//! ST 0903.6 VTargetPack encode: `write_pack` and `encoded_len`.

use super::model::VTargetPack;
use crate::error::KlvEncodeError;

/// Encode a single VTargetPack into `out`. Returns bytes written.
///
/// Fields are emitted in ascending tag order (1, 2, 3, ..., 23, 101,
/// 104, 105, 106, 107), then any preserved `unknown` tags last per
/// ST 0107.5 §6.
pub(crate) fn write_pack(pack: &VTargetPack, out: &mut Vec<u8>) -> Result<usize, KlvEncodeError> {
    use crate::klv::length::{write_ber, write_ber_oid};
    use crate::klv::st0903::emit::{emit_imapb_n, emit_tlv, emit_var};

    let start = out.len();

    // 1. BER-OID Target ID (5 bytes covers up to u32::MAX).
    let mut buf = [0u8; 5];
    let n = write_ber_oid(pack.target_id, &mut buf)?;
    out.extend_from_slice(&buf[..n]);

    if let Some(v) = pack.centroid_pixel {
        emit_var(out, 1, v)?;
    }
    if let Some(v) = pack.bbox_top_left_pixel {
        emit_var(out, 2, v)?;
    }
    if let Some(v) = pack.bbox_bottom_right_pixel {
        emit_var(out, 3, v)?;
    }
    if let Some(v) = pack.priority {
        emit_tlv(out, 4, &[v])?;
    }
    if let Some(v) = pack.confidence_level {
        emit_tlv(out, 5, &[v])?;
    }
    if let Some(v) = pack.history {
        emit_var(out, 6, v as u32)?;
    }
    if let Some(v) = pack.percentage_of_target_pixels {
        emit_tlv(out, 7, &[v])?;
    }
    if let Some(v) = pack.target_color {
        emit_tlv(out, 8, &v)?;
    }
    if let Some(v) = pack.target_intensity {
        emit_var(out, 9, v)?;
    }

    // IMAPB fields. Tags 10/11/13/14/15/16 use 3-byte IMAPB per
    // §10.2.2.11/.12/.14/.15/.16/.17 over [-19.2°, 19.2°]. Tag 12
    // uses 2-byte IMAPB per §10.2.2.13 over [-900 m, 19000 m].
    if let Some(v) = pack.centroid_lat_offset {
        emit_imapb_n(out, 10, v, -19.2, 19.2, 3)?;
    }
    if let Some(v) = pack.centroid_lon_offset {
        emit_imapb_n(out, 11, v, -19.2, 19.2, 3)?;
    }
    if let Some(v) = pack.centroid_hae {
        emit_imapb_n(out, 12, v, -900.0, 19000.0, 2)?;
    }
    if let Some(v) = pack.bbox_top_left_lat_offset {
        emit_imapb_n(out, 13, v, -19.2, 19.2, 3)?;
    }
    if let Some(v) = pack.bbox_top_left_lon_offset {
        emit_imapb_n(out, 14, v, -19.2, 19.2, 3)?;
    }
    if let Some(v) = pack.bbox_bottom_right_lat_offset {
        emit_imapb_n(out, 15, v, -19.2, 19.2, 3)?;
    }
    if let Some(v) = pack.bbox_bottom_right_lon_offset {
        emit_imapb_n(out, 16, v, -19.2, 19.2, 3)?;
    }

    if let Some(ref bytes) = pack.target_location {
        emit_tlv(out, 17, bytes)?;
    }
    if let Some(ref bytes) = pack.geospatial_contour_series {
        emit_tlv(out, 18, bytes)?;
    }
    if let Some(v) = pack.centroid_pix_row {
        emit_var(out, 19, v)?;
    }
    if let Some(v) = pack.centroid_pix_col {
        emit_var(out, 20, v)?;
    }
    if let Some(v) = pack.algorithm_id {
        emit_var(out, 22, v)?;
    }
    if let Some(v) = pack.detection_status {
        emit_tlv(out, 23, &[v])?;
    }
    if let Some(ref bytes) = pack.vmask {
        emit_tlv(out, 101, bytes)?;
    }
    if let Some(ref bytes) = pack.vtracker {
        emit_tlv(out, 104, bytes)?;
    }
    if let Some(ref bytes) = pack.vchip {
        emit_tlv(out, 105, bytes)?;
    }
    if let Some(ref bytes) = pack.vchip_series {
        emit_tlv(out, 106, bytes)?;
    }
    if let Some(ref bytes) = pack.vobject_series {
        emit_tlv(out, 107, bytes)?;
    }

    // Unknown tags preserved last (ST 0107.5 §6). Tag IDs use multi-
    // byte BER-OID per ST 0107.5 §6.3.1 for values ≥ 128, so a future
    // ST 0903.7+ pack tag in the unknown bucket round-trips losslessly.
    // Tags 1..=107 (the §10.2 typed universe) are all ≤ 127 and encode
    // as a single byte, byte-identical to the pre-E5-followup emit.
    // `encoded_len` mirrors this via `ber_oid_len(field.tag)`.
    for field in &pack.unknown {
        let mut tag_buf = [0u8; 5]; // u32 fits in at most 5 BER-OID bytes
        let tag_n = write_ber_oid(field.tag, &mut tag_buf)?;
        out.extend_from_slice(&tag_buf[..tag_n]);
        let mut len_buf = [0u8; 9];
        let len_n = write_ber(field.value.len(), &mut len_buf)?;
        out.extend_from_slice(&len_buf[..len_n]);
        out.extend_from_slice(&field.value);
    }

    Ok(out.len() - start)
}

/// Number of bytes `pack` would occupy when encoded. Mirrors
/// `write_pack`'s field-by-field structure.
pub(crate) fn encoded_len(pack: &VTargetPack) -> usize {
    use crate::klv::length::{ber_len, ber_oid_len};
    use crate::klv::st0903::var_uint::var_u32_len;

    fn tlv_len(value_len: usize) -> usize {
        1 /* tag */ + ber_len(value_len) + value_len
    }

    let mut total = ber_oid_len(pack.target_id);
    if let Some(v) = pack.centroid_pixel {
        total += tlv_len(var_u32_len(v));
    }
    if let Some(v) = pack.bbox_top_left_pixel {
        total += tlv_len(var_u32_len(v));
    }
    if let Some(v) = pack.bbox_bottom_right_pixel {
        total += tlv_len(var_u32_len(v));
    }
    if pack.priority.is_some() {
        total += tlv_len(1);
    }
    if pack.confidence_level.is_some() {
        total += tlv_len(1);
    }
    if let Some(v) = pack.history {
        total += tlv_len(var_u32_len(v as u32));
    }
    if pack.percentage_of_target_pixels.is_some() {
        total += tlv_len(1);
    }
    if pack.target_color.is_some() {
        total += tlv_len(3);
    }
    if let Some(v) = pack.target_intensity {
        total += tlv_len(var_u32_len(v));
    }
    if pack.centroid_lat_offset.is_some() {
        total += tlv_len(3);
    }
    if pack.centroid_lon_offset.is_some() {
        total += tlv_len(3);
    }
    if pack.centroid_hae.is_some() {
        total += tlv_len(2);
    }
    if pack.bbox_top_left_lat_offset.is_some() {
        total += tlv_len(3);
    }
    if pack.bbox_top_left_lon_offset.is_some() {
        total += tlv_len(3);
    }
    if pack.bbox_bottom_right_lat_offset.is_some() {
        total += tlv_len(3);
    }
    if pack.bbox_bottom_right_lon_offset.is_some() {
        total += tlv_len(3);
    }
    if let Some(ref b) = pack.target_location {
        total += tlv_len(b.len());
    }
    if let Some(ref b) = pack.geospatial_contour_series {
        total += tlv_len(b.len());
    }
    if let Some(v) = pack.centroid_pix_row {
        total += tlv_len(var_u32_len(v));
    }
    if let Some(v) = pack.centroid_pix_col {
        total += tlv_len(var_u32_len(v));
    }
    if let Some(v) = pack.algorithm_id {
        total += tlv_len(var_u32_len(v));
    }
    if pack.detection_status.is_some() {
        total += tlv_len(1);
    }
    if let Some(ref b) = pack.vmask {
        total += tlv_len(b.len());
    }
    if let Some(ref b) = pack.vtracker {
        total += tlv_len(b.len());
    }
    if let Some(ref b) = pack.vchip {
        total += tlv_len(b.len());
    }
    if let Some(ref b) = pack.vchip_series {
        total += tlv_len(b.len());
    }
    if let Some(ref b) = pack.vobject_series {
        total += tlv_len(b.len());
    }
    // Unknown tags use BER-OID tag + BER length + value (mirrors
    // `write_pack`). For tags ≤ 127 (the §10.2 typed universe),
    // `ber_oid_len(tag) == 1` so this collapses to the same byte
    // count as the pre-E5-followup `tlv_len(value.len())`.
    for field in &pack.unknown {
        total += ber_oid_len(field.tag) + ber_len(field.value.len()) + field.value.len();
    }
    total
}
