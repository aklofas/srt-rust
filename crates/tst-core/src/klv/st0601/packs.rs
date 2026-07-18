//! ST 0601 pack & list substrate (WP-C) — parse/emit for the tags whose
//! wire value is a small positional structure (a "Defined-Length Pack" /
//! "Variable-Length Pack", ST 0107 terminology) or a flat list, rather
//! than one scalar. Dispatch lives in `decode.rs`/`encode.rs` via the
//! `Encoding::Pack` marker (`tags.rs`); this module owns the per-tag wire
//! shape only.
//!
//! Tags covered here (Task C2 — the simple DLP packs): 81 (Image Horizon
//! Pixels), 115 (Control Command, MULTI-INSTANCE), 116 (Control Command
//! Verification List), 121 (Active Wavelength List), 127 (Sensor Frame
//! Rate Pack), 143 (Metadata Substream Id). The remaining WP-C pack tags
//! (122/128/130/138/140/141/142) and Tag 102 (SDCC-FLP, `klv::st1010`)
//! land in later WP-C tasks.

use alloc::borrow::ToOwned;
use alloc::string::String;
use alloc::vec::Vec;

use crate::error::{KlvDecodeError, KlvEncodeError, KlvFieldError};
use crate::klv::length::{
    ber_len, ber_oid_len_u64, read_ber, read_ber_oid_u64, read_var_uint, var_uint_min_len,
    write_ber, write_ber_oid_u64, write_var_uint_min,
};

use super::mapping::{decode_fixed_range, encode_fixed_range};
use super::model::OutOfRangePolicy;
use super::tags::LinearRange;

const LAT_RANGE: LinearRange = LinearRange {
    signed: true,
    byte_length: 4,
    min: -90.0,
    max: 90.0,
};
const LON_RANGE: LinearRange = LinearRange {
    signed: true,
    byte_length: 4,
    min: -180.0,
    max: 180.0,
};

/// Map a substrate [`KlvDecodeError`] (BER / BER-OID framing failure) to
/// the [`KlvFieldError`] this module's parse functions return — every
/// framing failure means the same thing at this level: the pack could
/// not be parsed. Sibling of `klv::st1010::substrate_err`.
fn truncated(tag: u32) -> impl FnOnce(KlvDecodeError) -> KlvFieldError {
    move |_| KlvFieldError::TruncatedField { tag }
}

/// Item 81: Image Horizon Pixels (ST 0601.19 §8.81) — screen-space
/// horizon line endpoints as percentages of image width/height, plus an
/// optional geodetic pair for each endpoint.
///
/// Wire shape (DLP, ST 0107 truncatable-trailing convention): 4
/// mandatory percentage bytes, then 0-4 further int32 linear-mapped
/// fields in strict order (`start_lat`, `start_lon`, `end_lat`,
/// `end_lon`) — the pack may end after any of them. A wire int32 exactly
/// at `INT_MIN` (`0x80000000`) on any of the four trailing fields is the
/// item's spec-defined "error" indicator (§8.81): that field decodes to
/// `None` just like a truncated-away field, but the byte position IS
/// consumed (parsing continues to the next field).
///
/// **Encode semantics:** a `None` field that lies BEFORE the last `Some`
/// field (in wire order) is re-encoded as the `INT_MIN` sentinel — the
/// spec's own error indicator, which decode maps straight back to `None`
/// — so a later `Some` field is never silently dropped just because an
/// earlier optional field is absent. Only a `None` AFTER the last `Some`
/// (or all four `None`) truncates the pack there, matching decode's own
/// truncation semantics: there is no wire difference between "never
/// sent" and "sentinel would be redundant here" once the pack has ended.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ImageHorizonPixels {
    pub x0_pct: u8,
    pub y0_pct: u8,
    pub x1_pct: u8,
    pub y1_pct: u8,
    pub start_lat_deg: Option<f64>,
    pub start_lon_deg: Option<f64>,
    pub end_lat_deg: Option<f64>,
    pub end_lon_deg: Option<f64>,
}

/// The four optional trailing fields of [`ImageHorizonPixels`], in wire
/// order, paired with the `LinearRange` each one decodes/encodes with.
/// Shared between [`parse_image_horizon`], [`emit_image_horizon`], and
/// [`image_horizon_len`] so the truncation-stop order can't drift between
/// them.
const IMAGE_HORIZON_OPTIONAL_RANGES: [LinearRange; 4] =
    [LAT_RANGE, LON_RANGE, LAT_RANGE, LON_RANGE];

fn image_horizon_optional_values(h: &ImageHorizonPixels) -> [Option<f64>; 4] {
    [
        h.start_lat_deg,
        h.start_lon_deg,
        h.end_lat_deg,
        h.end_lon_deg,
    ]
}

fn image_horizon_optional_setters() -> [fn(&mut ImageHorizonPixels, Option<f64>); 4] {
    [
        |h, v| h.start_lat_deg = v,
        |h, v| h.start_lon_deg = v,
        |h, v| h.end_lat_deg = v,
        |h, v| h.end_lon_deg = v,
    ]
}

pub(crate) fn parse_image_horizon(bytes: &[u8]) -> Result<ImageHorizonPixels, KlvFieldError> {
    if bytes.len() < 4 {
        return Err(KlvFieldError::TruncatedField { tag: 81 });
    }
    let mut h = ImageHorizonPixels {
        x0_pct: bytes[0],
        y0_pct: bytes[1],
        x1_pct: bytes[2],
        y1_pct: bytes[3],
        ..ImageHorizonPixels::default()
    };
    let rest = &bytes[4..];
    let setters = image_horizon_optional_setters();
    let mut offset = 0usize;
    for (range, setter) in IMAGE_HORIZON_OPTIONAL_RANGES.into_iter().zip(setters) {
        if offset + 4 > rest.len() {
            break; // clean truncation — no more optional fields on the wire
        }
        let v = decode_fixed_range(&range, 81, &rest[offset..offset + 4])?;
        setter(&mut h, v);
        offset += 4;
    }
    if offset != rest.len() {
        // Leftover bytes that don't align to a full 4-byte optional
        // field — not a clean truncation, matches the malformed-scalar
        // policy for over-long values.
        return Err(KlvFieldError::InvalidLength {
            tag: 81,
            expected: 4 + offset,
            got: bytes.len(),
        });
    }
    Ok(h)
}

/// Index of the last `Some` among the four optional trailing fields, or
/// `None` if all four are absent. Every optional field up to and
/// including this index gets a wire slot (sentinel-filled where the
/// field itself is `None`); shared by [`image_horizon_len`] and
/// [`emit_image_horizon`] so their stopping point can't drift apart.
fn image_horizon_last_some(h: &ImageHorizonPixels) -> Option<usize> {
    image_horizon_optional_values(h)
        .iter()
        .rposition(|v| v.is_some())
}

pub(crate) fn image_horizon_len(h: &ImageHorizonPixels) -> usize {
    match image_horizon_last_some(h) {
        Some(last) => 4 + 4 * (last + 1),
        None => 4,
    }
}

pub(crate) fn emit_image_horizon(
    h: &ImageHorizonPixels,
    out: &mut Vec<u8>,
) -> Result<(), KlvEncodeError> {
    out.push(h.x0_pct);
    out.push(h.y0_pct);
    out.push(h.x1_pct);
    out.push(h.y1_pct);
    let Some(last) = image_horizon_last_some(h) else {
        return Ok(()); // no optional fields at all
    };
    for (i, (value, range)) in image_horizon_optional_values(h)
        .into_iter()
        .zip(IMAGE_HORIZON_OPTIONAL_RANGES)
        .enumerate()
    {
        if i > last {
            break; // trailing None(s) after the last Some — clean truncation
        }
        let mut buf = [0u8; 4];
        match value {
            Some(v) => encode_fixed_range(&range, 81, v, &mut buf, OutOfRangePolicy::Error)?,
            // Interior None (before the last Some): fill with the
            // spec's own INT_MIN "error" indicator (§8.81) rather than
            // dropping this slot — decode maps it straight back to
            // `None`, so the round trip is lossless for the Some
            // fields that follow.
            None => buf = i32::MIN.to_be_bytes(),
        }
        out.extend_from_slice(&buf);
    }
    Ok(())
}

/// Item 115: Control Command (ST 0601.19 §8.115) — one command sent to
/// the platform. MULTI-INSTANCE per ST 0601.19 Table 1 ("Multiples
/// Allowed" = Yes): every wire occurrence of Tag 115 appends one
/// `ControlCommand` to `UasDatalinkLs::control_commands` rather than
/// overwriting a single field.
#[derive(Debug, Clone, PartialEq)]
pub struct ControlCommand {
    /// BER-OID command id.
    pub id: u64,
    /// Command text, UTF-8, at most 127 bytes.
    pub command: String,
    /// Time the command was issued/executed, microseconds — MISB
    /// var-length truncatable trailing field (present iff the wire pack
    /// has trailing bytes after `command`).
    pub time_us: Option<u64>,
}

/// Wire shape: `id (BER-OID, M)` + `string_len (BER) + command (UTF-8,
/// M, <=127 bytes)` + `[time_us (MISB var-length truncatable, 1..=8
/// bytes)]`.
pub(crate) fn parse_control_command(bytes: &[u8]) -> Result<ControlCommand, KlvFieldError> {
    let (id, rest) = read_ber_oid_u64(bytes).map_err(truncated(115))?;
    let (str_len, rest) = read_ber(rest).map_err(truncated(115))?;
    if str_len > 127 {
        return Err(KlvFieldError::InvalidLength {
            tag: 115,
            expected: 127,
            got: str_len,
        });
    }
    if rest.len() < str_len {
        return Err(KlvFieldError::TruncatedField { tag: 115 });
    }
    let (str_bytes, rest) = (&rest[..str_len], &rest[str_len..]);
    let command = core::str::from_utf8(str_bytes)
        .map_err(|_| KlvFieldError::InvalidUtf8 { tag: 115 })?
        .to_owned();
    let time_us = match rest.len() {
        0 => None,
        1..=8 => Some(read_var_uint(rest, 8, 115)?),
        got => {
            return Err(KlvFieldError::InvalidLength {
                tag: 115,
                expected: 8,
                got,
            });
        }
    };
    Ok(ControlCommand {
        id,
        command,
        time_us,
    })
}

/// Wire byte length [`emit_control_command`] would write for `cmd`,
/// without allocating — the sizing sibling used by the Tag 115
/// multi-instance special-case in `encode.rs`.
pub(crate) fn control_command_len(cmd: &ControlCommand) -> usize {
    ber_oid_len_u64(cmd.id)
        + ber_len(cmd.command.len())
        + cmd.command.len()
        + cmd.time_us.map(var_uint_min_len).unwrap_or(0)
}

pub(crate) fn emit_control_command(
    cmd: &ControlCommand,
    out: &mut Vec<u8>,
) -> Result<(), KlvEncodeError> {
    if cmd.command.len() > 127 {
        return Err(KlvEncodeError::StringTooLong { tag: 115, max: 127 });
    }
    let mut id_buf = [0u8; 10];
    let n = write_ber_oid_u64(cmd.id, &mut id_buf)?;
    out.extend_from_slice(&id_buf[..n]);
    let mut len_buf = [0u8; 9];
    let m = write_ber(cmd.command.len(), &mut len_buf)?;
    out.extend_from_slice(&len_buf[..m]);
    out.extend_from_slice(cmd.command.as_bytes());
    if let Some(t) = cmd.time_us {
        out.extend_from_slice(&write_var_uint_min(t));
    }
    Ok(())
}

/// Parse a Tag 116/121 value: a flat sequence of BER-OID ids, consumed
/// until the value bytes are exhausted (no per-item length prefix).
pub(crate) fn parse_id_list(bytes: &[u8], tag: u32) -> Result<Vec<u64>, KlvFieldError> {
    let mut ids = Vec::new();
    let mut rest = bytes;
    while !rest.is_empty() {
        let (id, r) = read_ber_oid_u64(rest).map_err(truncated(tag))?;
        ids.push(id);
        rest = r;
    }
    Ok(ids)
}

pub(crate) fn id_list_len(ids: &[u64]) -> usize {
    ids.iter().map(|&id| ber_oid_len_u64(id)).sum()
}

pub(crate) fn emit_id_list(ids: &[u64], out: &mut Vec<u8>) -> Result<(), KlvEncodeError> {
    let mut buf = [0u8; 10];
    for &id in ids {
        let n = write_ber_oid_u64(id, &mut buf)?;
        out.extend_from_slice(&buf[..n]);
    }
    Ok(())
}

/// Item 127: Sensor Frame Rate Pack (ST 0601.19 §8.127) — frame rate as a
/// numerator/denominator ratio; `denominator` defaults to 1 when the wire
/// pack ends after the numerator.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SensorFrameRate {
    /// BER-OID numerator.
    pub numerator: u64,
    /// BER-OID denominator; wire-absent means 1 (whole-number fps).
    pub denominator: u64,
}

impl SensorFrameRate {
    /// Frames per second as `numerator / denominator`.
    pub fn fps(&self) -> f64 {
        self.numerator as f64 / self.denominator as f64
    }
}

pub(crate) fn parse_sensor_frame_rate(bytes: &[u8]) -> Result<SensorFrameRate, KlvFieldError> {
    let (numerator, rest) = read_ber_oid_u64(bytes).map_err(truncated(127))?;
    let denominator = if rest.is_empty() {
        1
    } else {
        let (d, rest2) = read_ber_oid_u64(rest).map_err(truncated(127))?;
        if !rest2.is_empty() {
            return Err(KlvFieldError::InvalidLength {
                tag: 127,
                expected: bytes.len() - rest2.len(),
                got: bytes.len(),
            });
        }
        d
    };
    Ok(SensorFrameRate {
        numerator,
        denominator,
    })
}

/// Canonical wire length: the denominator is only emitted when it isn't
/// the default (1) — matches [`emit_sensor_frame_rate`].
pub(crate) fn sensor_frame_rate_len(fr: &SensorFrameRate) -> usize {
    ber_oid_len_u64(fr.numerator)
        + if fr.denominator == 1 {
            0
        } else {
            ber_oid_len_u64(fr.denominator)
        }
}

pub(crate) fn emit_sensor_frame_rate(
    fr: &SensorFrameRate,
    out: &mut Vec<u8>,
) -> Result<(), KlvEncodeError> {
    let mut buf = [0u8; 10];
    let n = write_ber_oid_u64(fr.numerator, &mut buf)?;
    out.extend_from_slice(&buf[..n]);
    if fr.denominator != 1 {
        let n2 = write_ber_oid_u64(fr.denominator, &mut buf)?;
        out.extend_from_slice(&buf[..n2]);
    }
    Ok(())
}

/// Item 143: Metadata Substream Id (ST 0601.19 §8.143) — identifies
/// which metadata substream this Local Set belongs to. Per §8.143, `uuid`
/// is REQUIRED when `local_id == 0` and OMITTED when `local_id > 0`; this
/// decoder is lenient and stores whatever combination the wire carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetadataSubstreamId {
    /// BER-OID local substream id.
    pub local_id: u64,
    /// RFC 4122 UUID bytes, present per the §8.143 rule above.
    pub uuid: Option<[u8; 16]>,
}

pub(crate) fn parse_metadata_substream_id(
    bytes: &[u8],
) -> Result<MetadataSubstreamId, KlvFieldError> {
    let (local_id, rest) = read_ber_oid_u64(bytes).map_err(truncated(143))?;
    let uuid = match rest.len() {
        0 => None,
        16 => {
            let mut u = [0u8; 16];
            u.copy_from_slice(rest);
            Some(u)
        }
        got => {
            return Err(KlvFieldError::InvalidLength {
                tag: 143,
                expected: 16,
                got,
            });
        }
    };
    Ok(MetadataSubstreamId { local_id, uuid })
}

pub(crate) fn metadata_substream_id_len(ms: &MetadataSubstreamId) -> usize {
    ber_oid_len_u64(ms.local_id) + if ms.uuid.is_some() { 16 } else { 0 }
}

pub(crate) fn emit_metadata_substream_id(
    ms: &MetadataSubstreamId,
    out: &mut Vec<u8>,
) -> Result<(), KlvEncodeError> {
    let mut buf = [0u8; 10];
    let n = write_ber_oid_u64(ms.local_id, &mut buf)?;
    out.extend_from_slice(&buf[..n]);
    if let Some(uuid) = ms.uuid {
        out.extend_from_slice(&uuid);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_horizon_truncated_optional_bytes_error() {
        // 4 mandatory + 2 stray bytes: not a clean truncation (doesn't
        // align to a 4-byte optional field boundary).
        let err = parse_image_horizon(&[0, 0, 0, 0, 0xAA, 0xBB]).unwrap_err();
        assert!(matches!(err, KlvFieldError::InvalidLength { tag: 81, .. }));
    }

    #[test]
    fn image_horizon_too_short_for_mandatory_fields_error() {
        let err = parse_image_horizon(&[0, 0, 0]).unwrap_err();
        assert!(matches!(err, KlvFieldError::TruncatedField { tag: 81 }));
    }

    #[test]
    fn image_horizon_full_pack_round_trips() {
        let h = ImageHorizonPixels {
            x0_pct: 10,
            y0_pct: 20,
            x1_pct: 30,
            y1_pct: 40,
            start_lat_deg: Some(10.0),
            start_lon_deg: Some(-20.0),
            end_lat_deg: Some(30.0),
            end_lon_deg: Some(-40.0),
        };
        assert_eq!(image_horizon_len(&h), 20);
        let mut buf = Vec::new();
        emit_image_horizon(&h, &mut buf).unwrap();
        assert_eq!(buf.len(), 20);
        let back = parse_image_horizon(&buf).unwrap();
        assert_eq!(back.x0_pct, 10);
        assert!((back.start_lat_deg.unwrap() - 10.0).abs() < 1e-3);
        assert!((back.end_lon_deg.unwrap() - (-40.0)).abs() < 1e-3);
    }

    /// A spec-legal wire can carry the INT_MIN "error" indicator
    /// (§8.81) for an EARLIER optional field while carrying valid data
    /// for a LATER one — decode must not lose the later fields, and
    /// re-encode must reproduce the exact same wire bytes (sentinel
    /// fill, not truncation).
    #[test]
    fn image_horizon_sentinel_then_valid_data_round_trips_byte_identical() {
        let mut wire = vec![10u8, 20, 30, 40];
        wire.extend_from_slice(&i32::MIN.to_be_bytes()); // start_lat: sentinel
        let mut buf = [0u8; 4];
        encode_fixed_range(&LON_RANGE, 81, -20.0, &mut buf, OutOfRangePolicy::Error).unwrap();
        wire.extend_from_slice(&buf); // start_lon: valid
        encode_fixed_range(&LAT_RANGE, 81, 30.0, &mut buf, OutOfRangePolicy::Error).unwrap();
        wire.extend_from_slice(&buf); // end_lat: valid
        encode_fixed_range(&LON_RANGE, 81, -40.0, &mut buf, OutOfRangePolicy::Error).unwrap();
        wire.extend_from_slice(&buf); // end_lon: valid

        let h = parse_image_horizon(&wire).unwrap();
        assert_eq!((h.x0_pct, h.y0_pct, h.x1_pct, h.y1_pct), (10, 20, 30, 40));
        assert_eq!(h.start_lat_deg, None, "INT_MIN sentinel decodes to None");
        assert!((h.start_lon_deg.unwrap() - (-20.0)).abs() < 1e-6);
        assert!((h.end_lat_deg.unwrap() - 30.0).abs() < 1e-6);
        assert!((h.end_lon_deg.unwrap() - (-40.0)).abs() < 1e-6);

        let mut re_encoded = Vec::new();
        emit_image_horizon(&h, &mut re_encoded).unwrap();
        assert_eq!(
            re_encoded, wire,
            "sentinel-then-data pack must re-encode byte-identical, not truncated"
        );
    }

    /// In-memory record with interior `None` gaps (not derived from a
    /// decode) must still round-trip its `Some` fields intact through
    /// encode -> decode.
    #[test]
    fn image_horizon_interior_none_round_trips_via_encode_decode() {
        let h = ImageHorizonPixels {
            x0_pct: 1,
            y0_pct: 2,
            x1_pct: 3,
            y1_pct: 4,
            start_lat_deg: None,
            start_lon_deg: Some(45.0),
            end_lat_deg: None,
            end_lon_deg: Some(-90.0),
        };
        let mut buf = Vec::new();
        emit_image_horizon(&h, &mut buf).unwrap();
        assert_eq!(buf.len(), image_horizon_len(&h));
        assert_eq!(
            buf.len(),
            20,
            "all 4 optional slots present up to the last Some"
        );
        let back = parse_image_horizon(&buf).unwrap();
        assert_eq!(back.start_lat_deg, None);
        assert!((back.start_lon_deg.unwrap() - 45.0).abs() < 1e-6);
        assert_eq!(back.end_lat_deg, None);
        assert!((back.end_lon_deg.unwrap() - (-90.0)).abs() < 1e-6);
    }

    #[test]
    fn control_command_over_length_string_rejected() {
        let cmd = ControlCommand {
            id: 1,
            command: "x".repeat(128),
            time_us: None,
        };
        let mut buf = Vec::new();
        let err = emit_control_command(&cmd, &mut buf).unwrap_err();
        assert!(matches!(
            err,
            KlvEncodeError::StringTooLong { tag: 115, max: 127 }
        ));
    }

    #[test]
    fn control_command_with_time_us_round_trips() {
        let cmd = ControlCommand {
            id: 200,
            command: "abc".into(),
            time_us: Some(1_700_000_000_000_000),
        };
        let mut buf = Vec::new();
        emit_control_command(&cmd, &mut buf).unwrap();
        assert_eq!(buf.len(), control_command_len(&cmd));
        let back = parse_control_command(&buf).unwrap();
        assert_eq!(back, cmd);
    }

    #[test]
    fn id_list_empty_and_round_trip() {
        assert_eq!(parse_id_list(&[], 116).unwrap(), Vec::<u64>::new());
        let ids = vec![0u64, 3, 300];
        let mut buf = Vec::new();
        emit_id_list(&ids, &mut buf).unwrap();
        assert_eq!(buf.len(), id_list_len(&ids));
        assert_eq!(parse_id_list(&buf, 116).unwrap(), ids);
    }

    #[test]
    fn sensor_frame_rate_fps_computes_ratio() {
        let fr = SensorFrameRate {
            numerator: 60000,
            denominator: 1001,
        };
        assert!((fr.fps() - 59.940_059_940_06).abs() < 1e-9);
    }

    #[test]
    fn sensor_frame_rate_explicit_denominator_one_canonicalizes_away() {
        // Wire explicitly encodes denominator=1; re-encode canonicalizes
        // to the shorter numerator-only form (documented behavior).
        let bytes = [0x1E, 0x01]; // 30, 1
        let fr = parse_sensor_frame_rate(&bytes).unwrap();
        assert_eq!((fr.numerator, fr.denominator), (30, 1));
        let mut buf = Vec::new();
        emit_sensor_frame_rate(&fr, &mut buf).unwrap();
        assert_eq!(buf, vec![0x1E]);
    }

    #[test]
    fn metadata_substream_id_bad_uuid_length_rejected() {
        let err = parse_metadata_substream_id(&[0x00, 0xAA, 0xBB]).unwrap_err();
        assert!(matches!(
            err,
            KlvFieldError::InvalidLength {
                tag: 143,
                expected: 16,
                got: 2,
            }
        ));
    }

    #[test]
    fn metadata_substream_id_local_id_only_round_trips() {
        let ms = MetadataSubstreamId {
            local_id: 5,
            uuid: None,
        };
        let mut buf = Vec::new();
        emit_metadata_substream_id(&ms, &mut buf).unwrap();
        assert_eq!(buf, vec![0x05]);
        assert_eq!(parse_metadata_substream_id(&buf).unwrap(), ms);
    }
}
