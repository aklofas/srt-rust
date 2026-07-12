//! MISB ST 0604 MISP timestamps for compressed Motion Imagery.
//!
//! Builds and extracts the ST 0604.6 Precision / Nano Precision Time
//! Stamp carried in an H.264 / H.265 `user_data_unregistered` SEI
//! message (§7, §11.1, §12.1/§12.2). The 28-byte payload is a 16-byte
//! identifier, a 1-byte MISB ST 0603 Time Status, and an 11-byte
//! "Modified" timestamp (8-byte big-endian value with a `0xFF` guard
//! byte after each 2-byte group, §7.4 Table 2).
//!
//! Out of scope (see `docs/project/deferred-features.md`):
//! H.262/MPEG-2 `user_data` carriage (§10), the Commercial Time Stamp
//! (`pic_timing` / `time_code` SEI, §11.2/§12.3), and AV1 / H.266
//! (ST 0604 defines no carriage for them).

use crate::mpegts::mux::VideoCodec;

/// ST 0604.6 §7.1 Table 1 — H.262/H.264 Precision Time Stamp Identifier.
pub const MISP_MICROSEC_ID_H264: [u8; 16] = *b"MISPmicrosectime";
/// ST 0604.6 §7.2 — H.265 Precision (microsecond) Time Stamp Identifier.
pub const MISP_MICROSEC_ID_H265: [u8; 16] = [
    0xa8, 0x68, 0x7d, 0xd4, 0xd7, 0x59, 0x37, 0x58,
    0xa5, 0xce, 0xf0, 0x33, 0x8b, 0x65, 0x45, 0xf1,
];
/// ST 0604.6 §8.1 — H.265 Nano Precision Time Stamp Identifier.
pub const MISP_NANOSEC_ID_H265: [u8; 16] = [
    0xcf, 0x84, 0x82, 0x78, 0xee, 0x23, 0x30, 0x6c,
    0x92, 0x65, 0xe8, 0xfe, 0xf2, 0x2f, 0xb8, 0xb8,
];

/// Which MISP time base a [`MispTimestamp`] carries.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MispTimeKind {
    /// Microseconds since the MISP epoch (ST 0603 Precision Time Stamp).
    Micro,
    /// Nanoseconds since the MISP epoch (ST 0603 Nano Precision Time
    /// Stamp). H.265-only per ST 0604.6 §12.2.
    Nano,
}

/// One MISP timestamp destined for (or extracted from) a video SEI.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MispTimestamp {
    pub kind: MispTimeKind,
    /// MISB ST 0603 Time Status byte (see [`crate::klv::st0605`] for the
    /// same byte in Class 0 packs).
    pub time_status: u8,
    /// Micro: microseconds since the MISP epoch. Nano: nanoseconds.
    pub value: u64,
}

impl MispTimestamp {
    /// Microsecond-precision timestamp (valid for H.264 and H.265).
    pub fn micros(value_us: u64, time_status: u8) -> Self {
        Self { kind: MispTimeKind::Micro, time_status, value: value_us }
    }

    /// Nanosecond-precision timestamp (H.265-only per ST 0604.6 §12.2).
    pub fn nanos(value_ns: u64, time_status: u8) -> Self {
        Self { kind: MispTimeKind::Nano, time_status, value: value_ns }
    }
}

/// Why a MISP SEI could not be built or spliced.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum MispTimeError {
    /// The Nano Precision Time Stamp is defined for H.265 only
    /// (ST 0604.6 §12.2); H.264 carries the microsecond form.
    #[error("nano-precision MISP timestamp is H.265-only (ST 0604.6 §12.2), not {codec:?}")]
    NanoUnsupportedForCodec { codec: VideoCodec },
    /// ST 0604 defines SEI timestamp carriage for H.264 and H.265 only.
    #[error("ST 0604 defines no MISP SEI carriage for {codec:?}")]
    UnsupportedCodec { codec: VideoCodec },
    /// The access unit contains no VCL NAL to anchor the SEI in front of.
    #[error("access unit contains no VCL NAL unit to place the MISP SEI before")]
    NoVclNal,
}

/// Why a present MISP SEI payload failed to parse (absence is `Ok(None)`).
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum MispTimeExtractError {
    /// A matched MISP SEI message was shorter than the mandatory 28 bytes.
    #[error("MISP SEI payload truncated (need 28 bytes)")]
    TruncatedSei,
    /// One of the ST 0604.6 §7.4 `0xFF` guard bytes was absent.
    #[error("MISP SEI modified-timestamp guard byte is not 0xFF")]
    BadGuardByte,
}

/// The 16-byte `uuid_iso_iec_11578` identifier for a codec + kind combo.
pub fn identifier_for(
    codec: VideoCodec,
    kind: MispTimeKind,
) -> Result<&'static [u8; 16], MispTimeError> {
    match (codec, kind) {
        (VideoCodec::H264, MispTimeKind::Micro) => Ok(&MISP_MICROSEC_ID_H264),
        (VideoCodec::H264, MispTimeKind::Nano) => {
            Err(MispTimeError::NanoUnsupportedForCodec { codec })
        }
        (VideoCodec::H265, MispTimeKind::Micro) => Ok(&MISP_MICROSEC_ID_H265),
        (VideoCodec::H265, MispTimeKind::Nano) => Ok(&MISP_NANOSEC_ID_H265),
        (VideoCodec::H266 | VideoCodec::Av1, _) => {
            Err(MispTimeError::UnsupportedCodec { codec })
        }
    }
}

/// Assemble the 28-byte ST 0604.6 SEI payload (Table 2): identifier,
/// Time Status, then the 8-byte big-endian value as four 2-byte groups
/// with a `0xFF` guard byte after each of the first three groups.
pub(crate) fn sei_payload(
    codec: VideoCodec,
    ts: &MispTimestamp,
) -> Result<[u8; 28], MispTimeError> {
    let id = identifier_for(codec, ts.kind)?;
    let v = ts.value.to_be_bytes();
    let mut out = [0u8; 28];
    out[..16].copy_from_slice(id);
    out[16] = ts.time_status;
    out[17] = v[0];
    out[18] = v[1];
    out[19] = 0xFF;
    out[20] = v[2];
    out[21] = v[3];
    out[22] = 0xFF;
    out[23] = v[4];
    out[24] = v[5];
    out[25] = 0xFF;
    out[26] = v[6];
    out[27] = v[7];
    Ok(out)
}

/// Append `rbsp` to `out` with `emulation_prevention_three_byte`
/// insertion (H.264 §7.4.1 / H.265 §7.4.2): any byte value 0x00–0x03
/// that would follow two consecutive zero bytes gets a 0x03 inserted
/// before it. `out`'s existing tail does not count toward the zero run
/// (callers append the NAL header first — headers are non-zero here).
pub(crate) fn append_rbsp_escaped(out: &mut Vec<u8>, rbsp: &[u8]) {
    let mut zeros = 0u32;
    for &b in rbsp {
        if zeros >= 2 && b <= 0x03 {
            out.push(0x03);
            zeros = 0;
        }
        out.push(b);
        if b == 0 { zeros += 1 } else { zeros = 0 }
    }
}

/// Build the complete MISP-timestamp SEI NAL for one access unit.
///
/// Returns the bare NAL bytes (header + escaped RBSP, NO Annex-B start
/// code — the splice site supplies a 3-byte `00 00 01` prefix). H.264
/// gets `nal_unit_type=6` (`nal_ref_idc=0`); H.265 gets PREFIX_SEI
/// (type 39, `nuh_layer_id=0`, `nuh_temporal_id_plus1=1`). The SEI
/// message is `user_data_unregistered` (payloadType 5) with the fixed
/// 28-byte ST 0604 payload.
pub fn build_sei_nal(
    codec: VideoCodec,
    ts: &MispTimestamp,
) -> Result<Vec<u8>, MispTimeError> {
    let payload = sei_payload(codec, ts)?;
    // RBSP = payload_type(5) + payload_size(28) + payload + trailing 0x80.
    let mut rbsp = [0u8; 31];
    rbsp[0] = 0x05;
    rbsp[1] = 28;
    rbsp[2..30].copy_from_slice(&payload);
    rbsp[30] = 0x80;
    let mut out = Vec::with_capacity(2 + rbsp.len() + 4);
    match codec {
        VideoCodec::H264 => out.push(0x06),
        VideoCodec::H265 => out.extend_from_slice(&[0x4E, 0x01]),
        // sei_payload() above already rejected H.266 / AV1.
        VideoCodec::H266 | VideoCodec::Av1 => unreachable!("rejected by sei_payload"),
    }
    append_rbsp_escaped(&mut out, &rbsp);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mpegts::mux::VideoCodec;

    #[test]
    fn identifier_constants_match_st0604() {
        // §7.1 Table 1: ASCII "MISPmicrosectime".
        assert_eq!(&MISP_MICROSEC_ID_H264, b"MISPmicrosectime");
        // §7.2: a8687dd4-d759-3758-a5ce-f0338b6545f1
        assert_eq!(
            MISP_MICROSEC_ID_H265,
            [0xa8, 0x68, 0x7d, 0xd4, 0xd7, 0x59, 0x37, 0x58,
             0xa5, 0xce, 0xf0, 0x33, 0x8b, 0x65, 0x45, 0xf1]
        );
        // §8.1: cf848278-ee23-306c-9265-e8fef22fb8b8
        assert_eq!(
            MISP_NANOSEC_ID_H265,
            [0xcf, 0x84, 0x82, 0x78, 0xee, 0x23, 0x30, 0x6c,
             0x92, 0x65, 0xe8, 0xfe, 0xf2, 0x2f, 0xb8, 0xb8]
        );
    }

    #[test]
    fn payload_layout_matches_table_2() {
        // ST 0604.6 §7.4 Table 2: id(16) + status(1) + 2,2 FF 2,2 FF 2,2 FF 2,2.
        let ts = MispTimestamp::micros(0x0102_0304_0506_0708, 0x9F);
        let p = sei_payload(VideoCodec::H264, &ts).unwrap();
        assert_eq!(&p[..16], b"MISPmicrosectime");
        assert_eq!(p[16], 0x9F);
        assert_eq!(
            &p[17..28],
            &[0x01, 0x02, 0xFF, 0x03, 0x04, 0xFF, 0x05, 0x06, 0xFF, 0x07, 0x08]
        );
    }

    #[test]
    fn kind_codec_matrix() {
        let nano = MispTimestamp::nanos(1, 0x1F);
        assert!(matches!(
            sei_payload(VideoCodec::H264, &nano),
            Err(MispTimeError::NanoUnsupportedForCodec { .. })
        ));
        assert!(sei_payload(VideoCodec::H265, &nano).is_ok());
        let micro = MispTimestamp::micros(1, 0x1F);
        assert_eq!(&sei_payload(VideoCodec::H265, &micro).unwrap()[..16], &MISP_MICROSEC_ID_H265);
        for c in [VideoCodec::H266, VideoCodec::Av1] {
            assert!(matches!(
                sei_payload(c, &micro),
                Err(MispTimeError::UnsupportedCodec { .. })
            ));
        }
    }

    #[test]
    fn rbsp_escape_inserts_before_low_bytes() {
        for (raw, escaped) in [
            (&[0x00, 0x00, 0x00][..], &[0x00, 0x00, 0x03, 0x00][..]),
            (&[0x00, 0x00, 0x01][..], &[0x00, 0x00, 0x03, 0x01][..]),
            (&[0x00, 0x00, 0x02][..], &[0x00, 0x00, 0x03, 0x02][..]),
            (&[0x00, 0x00, 0x03][..], &[0x00, 0x00, 0x03, 0x03][..]),
            (&[0x00, 0x00, 0x04][..], &[0x00, 0x00, 0x04][..]),
            // Escape resets the zero-run: 00 00 00 00 -> 00 00 03 00 00.
            (&[0x00, 0x00, 0x00, 0x00][..], &[0x00, 0x00, 0x03, 0x00, 0x00][..]),
            (&[0xAA, 0x00, 0x00, 0x01, 0xBB][..], &[0xAA, 0x00, 0x00, 0x03, 0x01, 0xBB][..]),
        ] {
            let mut out = Vec::new();
            append_rbsp_escaped(&mut out, raw);
            assert_eq!(out, escaped, "raw={raw:02X?}");
        }
    }

    #[test]
    fn build_sei_nal_h264_golden() {
        let ts = MispTimestamp::micros(0x0102_0304_0506_0708, 0x9F);
        let nal = build_sei_nal(VideoCodec::H264, &ts).unwrap();
        // No escapes needed for this payload: header, type, size, 28, trailer.
        let mut expect = vec![0x06, 0x05, 28];
        expect.extend_from_slice(&sei_payload(VideoCodec::H264, &ts).unwrap());
        expect.push(0x80);
        assert_eq!(nal, expect);
    }

    #[test]
    fn build_sei_nal_h265_header() {
        let ts = MispTimestamp::nanos(7, 0x1F);
        let nal = build_sei_nal(VideoCodec::H265, &ts).unwrap();
        // nal_unit_type 39 (PREFIX_SEI) << 1 = 0x4E; layer 0, tid+1 = 1.
        assert_eq!(&nal[..2], &[0x4E, 0x01]);
        assert_eq!(nal[2], 0x05);
        assert_eq!(nal[3], 28);
        assert_eq!(&nal[4..20], &MISP_NANOSEC_ID_H265);
        assert_eq!(*nal.last().unwrap(), 0x80);
    }

    #[test]
    fn build_sei_nal_escapes_zero_run() {
        // status 0x00 + value MSBs 0x00 0x00 forms 00 00 00 inside the
        // payload -> the escaper MUST fire (identifier ends with 'e').
        let ts = MispTimestamp::micros(0x0000_0000_0000_0000, 0x00);
        let nal = build_sei_nal(VideoCodec::H264, &ts).unwrap();
        assert!(nal.len() > 3 + 28 + 1, "escapes must lengthen the NAL");
        // No unescaped start-code-like sequence may remain anywhere.
        assert!(!nal.windows(3).any(|w| w == [0, 0, 0] || w == [0, 0, 1] || w == [0, 0, 2]));
    }
}
