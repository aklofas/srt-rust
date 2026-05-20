//! ST 0903.6 VTargetPack decode: `read_pack` entry point.

use super::model::{PackEncoding, VTargetPack, VTargetPackError, pack_lookup};

/// Decode a single VTargetPack from `bytes`. Returns the decoded pack
/// and the number of bytes consumed.
///
/// Wire form per ST 0903.6 §10.2 Table 10:
/// - Leading BER-OID `targetId` (no Tag, per §10.2.2.1).
/// - Then a Local Set–encoded body where each field is
///   `[1-byte tag][BER short/long length][value bytes]`.
///
/// Unknown / deprecated tags (e.g. 21, 102, 103) are preserved in
/// `pack.unknown` per ST 0107.5 §6 future-proof skip rule.
pub(crate) fn read_pack(bytes: &[u8]) -> Result<(VTargetPack, usize), VTargetPackError> {
    use crate::klv::length::{read_ber, read_ber_oid};

    // 1. Read the leading BER-OID Target ID.
    let (target_id, rest) = read_ber_oid(bytes).map_err(|_| VTargetPackError::TruncatedTargetId)?;
    let header_consumed = bytes.len() - rest.len();

    let mut pack = VTargetPack {
        target_id,
        ..Default::default()
    };

    // 2. Walk the LS-encoded body. Each field is a single-byte tag
    //    (PACK_TAGS only uses 1..=107) + BER-encoded length + value.
    let mut cursor = rest;
    let mut consumed = header_consumed;
    while !cursor.is_empty() {
        let tag = cursor[0];
        cursor = &cursor[1..];
        consumed += 1;

        let (declared_len, after_len) =
            read_ber(cursor).map_err(|_| VTargetPackError::TruncatedField { tag })?;
        let len_consumed = cursor.len() - after_len.len();
        cursor = after_len;
        consumed += len_consumed;

        if cursor.len() < declared_len {
            return Err(VTargetPackError::LengthOverrun {
                tag,
                declared: declared_len,
                available: cursor.len(),
            });
        }
        let value = &cursor[..declared_len];
        cursor = &cursor[declared_len..];
        consumed += declared_len;

        decode_field(tag, value, &mut pack)?;
    }

    Ok((pack, consumed))
}

/// Dispatch a single TLV field's value bytes to the matching
/// `VTargetPack` field based on the spec's encoding for that tag.
/// Unknown / deprecated tags fall through to `pack.unknown` per
/// ST 0107.5 §6.
fn decode_field(tag: u8, value: &[u8], pack: &mut VTargetPack) -> Result<(), VTargetPackError> {
    use crate::klv::imapb::{ImapbParams, decode_imapb};
    use crate::klv::pack::OwnedRawField;
    use crate::klv::st0903::var_uint::read_var_u32;

    let Some(spec) = pack_lookup(tag) else {
        // ST 0107.5 §6 skip rule — preserve unknown / deprecated tags.
        pack.unknown.push(OwnedRawField {
            tag: tag as u32,
            value: value.to_vec(),
        });
        return Ok(());
    };

    match spec.encoding {
        PackEncoding::U8 => {
            if value.len() != 1 {
                return Err(VTargetPackError::InvalidLength {
                    tag,
                    expected: 1,
                    got: value.len(),
                });
            }
            let v = value[0];
            match tag {
                4 => pack.priority = Some(v),
                5 => pack.confidence_level = Some(v),
                7 => pack.percentage_of_target_pixels = Some(v),
                23 => pack.detection_status = Some(v),
                _ => unreachable!("U8 dispatch missing tag {tag}"),
            }
        }
        PackEncoding::VarUint { max_bytes } => {
            if value.is_empty() || value.len() > max_bytes as usize {
                return Err(VTargetPackError::InvalidLength {
                    tag,
                    expected: max_bytes as usize,
                    got: value.len(),
                });
            }
            // VarUint codec returns u32; per-tag downcasts handled below.
            let v = read_var_u32(value).map_err(|_| VTargetPackError::TruncatedField { tag })?;
            match tag {
                1 => pack.centroid_pixel = Some(v),
                2 => pack.bbox_top_left_pixel = Some(v),
                3 => pack.bbox_bottom_right_pixel = Some(v),
                6 => pack.history = Some(v as u16), // V2 caps at u16
                9 => pack.target_intensity = Some(v),
                19 => pack.centroid_pix_row = Some(v),
                20 => pack.centroid_pix_col = Some(v),
                22 => pack.algorithm_id = Some(v),
                _ => unreachable!("VarUint dispatch missing tag {tag}"),
            }
        }
        PackEncoding::U24Rgb => {
            if value.len() != 3 {
                return Err(VTargetPackError::InvalidLength {
                    tag,
                    expected: 3,
                    got: value.len(),
                });
            }
            pack.target_color = Some([value[0], value[1], value[2]]);
        }
        PackEncoding::ImapbF64 { min, max } => {
            // Tag 12 (`targetHae`) uses 2-byte IMAPB; all other IMAPB
            // pack tags use 3-byte IMAPB per §10.2.2.11–.17.
            let length = if tag == 12 { 2 } else { 3 };
            let params = ImapbParams { min, max, length };
            // A7: decode_imapb returns DecodedImapb (ST 1201.5 §7.2.2/.3
            // special values + bounds check). VTargetPack treats every
            // non-Value result as MalformedImapb — special-value
            // signaling at the per-target pack layer isn't a use case
            // the API surfaces today; callers needing to differentiate
            // can pattern-match the enum directly.
            let v = decode_imapb(&params, value)
                .map_err(|_| VTargetPackError::MalformedImapb { tag })?
                .value()
                .ok_or(VTargetPackError::MalformedImapb { tag })?;
            match tag {
                10 => pack.centroid_lat_offset = Some(v),
                11 => pack.centroid_lon_offset = Some(v),
                12 => pack.centroid_hae = Some(v),
                13 => pack.bbox_top_left_lat_offset = Some(v),
                14 => pack.bbox_top_left_lon_offset = Some(v),
                15 => pack.bbox_bottom_right_lat_offset = Some(v),
                16 => pack.bbox_bottom_right_lon_offset = Some(v),
                _ => unreachable!("ImapbF64 dispatch missing tag {tag}"),
            }
        }
        PackEncoding::RawBytes => {
            let bytes = value.to_vec();
            match tag {
                17 => pack.target_location = Some(bytes),
                18 => pack.geospatial_contour_series = Some(bytes),
                101 => pack.vmask = Some(bytes),
                104 => pack.vtracker = Some(bytes),
                105 => pack.vchip = Some(bytes),
                106 => pack.vchip_series = Some(bytes),
                107 => pack.vobject_series = Some(bytes),
                _ => unreachable!("RawBytes dispatch missing tag {tag}"),
            }
        }
    }
    Ok(())
}
