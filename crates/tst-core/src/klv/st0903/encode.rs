//! ST 0903 encode entry points: `encode`, `encode_to_vec`,
//! `encode_standalone`, `encode_to_vec_standalone`, `encoded_len_standalone`,
//! `encoded_len`.

use crate::error::KlvEncodeError;
use crate::klv::st0903::model::VmtiLs;
use crate::klv::st0903::{VMTI_LS_UL, vtarget_pack};
use alloc::vec::Vec;

/// Encode a VMTI Local Set **body** (no UL prefix, no outer BER length,
/// no Tag 1 checksum).
///
/// This is the **embedded-VMTI** entry point per ST 0903.6 §10 — used
/// when the VMTI LS rides inside another KLV carrier (most commonly
/// ST 0601 Tag 74). The Tag 1 checkSum is **always omitted** per
/// ST 0903.6-120 ("where the VMTI LS is embedded-VMTI, the VMTI LS
/// checkSum (Item 1) shall be omitted"). Any value the caller stored in
/// [`VmtiLs::checksum`] is silently dropped — the field exists for
/// decode-side fidelity only.
///
/// For **standalone-VMTI** carriage (separate KLV PID, wrapped as
/// `[VMTI_LS_UL:16][outer BER length][body][Tag 1 checksum TLV]`), use
/// [`encode_standalone`] / [`encode_to_vec_standalone`], which compute
/// the running 16-bit checksum per §10.1.1 and append Tag 1 last.
///
/// Fields are emitted in ascending tag order (2, 3, 4, 5, 6, 8, 9, 10,
/// 11, 12, 13, 101, 102, 103); Tag 7 (`motionImageryFrameNumber`) is
/// deprecated in v6 and never emitted. Preserved `unknown` tags are
/// appended last per ST 0107.5 §6 (single-byte tag IDs only — the
/// VMTI LS spec keeps tags ≤107).
///
/// Round-trip property: `decode(encode_to_vec(&ls)?)?` reproduces all
/// typed fields and preserved unknowns of `ls` (modulo IMAPB
/// quantization on `horizontal_fov` / `vertical_fov` and the dropped
/// `checksum` field). `field_errors` is a decode-time diagnostic and
/// is not emitted on encode.
///
/// # Errors
/// - [`KlvEncodeError::OutOfRange`] if `horizontal_fov` /
///   `vertical_fov` (or any per-target IMAPB float field) falls
///   outside its declared range.
/// - [`KlvEncodeError::RecordTooLarge`] if a TLV's declared length
///   would overflow BER encoding.
pub fn encode(ls: &VmtiLs, out: &mut Vec<u8>) -> Result<(), KlvEncodeError> {
    use crate::klv::length::write_ber;
    use crate::klv::st0903::emit::{emit_imapb_n, emit_tlv, emit_var};

    // Embedded-VMTI body: Tag 1 (checkSum) is omitted per ST 0903.6-120.
    // Tag 2 (precisionTimeStamp) first per ST 0903.4-14 — since Tag 1
    // is now absent, ascending-tag-order naturally places Tag 2 first.
    // Tag 7 (deprecated) is intentionally skipped (no struct field).
    if let Some(v) = ls.precision_time_stamp {
        emit_tlv(out, 2, &v.to_be_bytes())?;
    }
    if let Some(ref s) = ls.vmti_system_name {
        emit_tlv(out, 3, s.as_bytes())?;
    }
    if let Some(v) = ls.version_number {
        emit_var(out, 4, v as u32)?;
    }
    if let Some(v) = ls.total_targets_in_frame {
        emit_var(out, 5, v)?;
    }
    if let Some(v) = ls.num_targets_reported {
        emit_var(out, 6, v)?;
    }
    if let Some(v) = ls.frame_width {
        emit_var(out, 8, v)?;
    }
    if let Some(v) = ls.frame_height {
        emit_var(out, 9, v)?;
    }
    if let Some(ref s) = ls.source_sensor {
        emit_tlv(out, 10, s.as_bytes())?;
    }
    // Top-level FOV tags use IMAPB(0, 180, 2) per §10.1.11 + §10.1.12.
    if let Some(v) = ls.horizontal_fov {
        emit_imapb_n(out, 11, v, 0.0, 180.0, 2)?;
    }
    if let Some(v) = ls.vertical_fov {
        emit_imapb_n(out, 12, v, 0.0, 180.0, 2)?;
    }
    if let Some(ref bytes) = ls.miis_id {
        emit_tlv(out, 13, bytes)?;
    }

    // VTargetSeries (Tag 101). Each pack is BER-length-prefixed inside
    // the series payload (matches `decode_vtarget_series` framing).
    if !ls.targets.is_empty() {
        let mut series = Vec::new();
        for pack in &ls.targets {
            let mut pack_bytes = Vec::new();
            vtarget_pack::write_pack(pack, &mut pack_bytes)?;
            let mut len_buf = [0u8; 9];
            let len_n = write_ber(pack_bytes.len(), &mut len_buf)?;
            series.extend_from_slice(&len_buf[..len_n]);
            series.extend_from_slice(&pack_bytes);
        }
        emit_tlv(out, 101, &series)?;
    }

    if let Some(ref bytes) = ls.algorithm_series {
        emit_tlv(out, 102, bytes)?;
    }
    if let Some(ref bytes) = ls.ontology_series {
        emit_tlv(out, 103, bytes)?;
    }

    // Unknown tags last (preserves them per ST 0107.5 §6). Tag IDs use
    // multi-byte BER-OID encoding per ST 0107.5 §6.3.1 for values ≥ 128,
    // so a future ST 0903.7+ tag in the unknown bucket round-trips
    // losslessly. Tags 1..=103 (the §10.1 typed universe) are all ≤ 127
    // and encode as a single byte, byte-identical to the pre-E5 emit.
    // `encoded_len` mirrors this via `ber_oid_len(field.tag)`.
    use crate::klv::length::write_ber_oid;
    for field in &ls.unknown {
        let mut tag_buf = [0u8; 5]; // u32 fits in at most 5 BER-OID bytes
        let tag_n = write_ber_oid(field.tag, &mut tag_buf)?;
        out.extend_from_slice(&tag_buf[..tag_n]);
        let mut len_buf = [0u8; 9];
        let len_n = write_ber(field.value.len(), &mut len_buf)?;
        out.extend_from_slice(&len_buf[..len_n]);
        out.extend_from_slice(&field.value);
    }

    Ok(())
}

/// Encode a VMTI Local Set into a fresh `Vec<u8>`. Convenience over
/// [`encode`] when the caller has no pre-sized buffer.
///
/// # Errors
/// Returns the same [`KlvEncodeError`] variants as [`encode`].
pub fn encode_to_vec(ls: &VmtiLs) -> Result<Vec<u8>, KlvEncodeError> {
    let mut out = Vec::new();
    encode(ls, &mut out)?;
    Ok(out)
}

/// Encode a VMTI Local Set as a **standalone-VMTI** wire record:
/// `[VMTI_LS_UL:16][outer BER length][body][Tag 1 checkSum TLV]`.
///
/// Per ST 0903.4-17 / ST 0903.6-119, standalone-VMTI MUST place
/// Tag 1 last. Per ST 0903.6 §10.1.1, the Tag 1 value is the running
/// 16-bit unsigned summation of all bytes from the first byte of the
/// VMTI LS's UL through the last byte of the Tag 1 length (i.e. up to
/// but not including the 2-byte Tag 1 value itself). This function
/// computes the checksum from the assembled framing; any value the
/// caller stored in [`VmtiLs::checksum`] is ignored.
///
/// The function writes directly into `out`, returning the number of
/// bytes written. Use [`encode_to_vec_standalone`] when the caller
/// has no pre-sized buffer.
///
/// # Errors
/// - [`KlvEncodeError::BufferTooSmall`] if `out` is shorter than
///   [`encoded_len_standalone`].
/// - [`KlvEncodeError::OutOfRange`] / [`KlvEncodeError::RecordTooLarge`]
///   per [`encode`].
pub fn encode_standalone(ls: &VmtiLs, out: &mut [u8]) -> Result<usize, KlvEncodeError> {
    use crate::klv::checksum::checksum_running_sum_16;
    use crate::klv::length::{ber_len, write_ber};

    // Build the body (no Tag 1, no UL) into a temporary Vec — exactly
    // what `encode` produces today.
    let mut body: Vec<u8> = Vec::with_capacity(256);
    encode(ls, &mut body)?;

    // Tag 1 TLV is 4 bytes: tag (0x01) + length (0x02) + 2-byte value.
    const TAG1_TLV_LEN: usize = 4;
    let body_len_with_checksum = body.len() + TAG1_TLV_LEN;
    let outer_len_bytes = ber_len(body_len_with_checksum);
    let total = 16 + outer_len_bytes + body_len_with_checksum;

    if out.len() < total {
        return Err(KlvEncodeError::BufferTooSmall {
            needed: total,
            got: out.len(),
        });
    }

    // 1) UL
    out[..16].copy_from_slice(&VMTI_LS_UL);
    // 2) Outer BER length (covers body + Tag 1 TLV)
    let written = write_ber(body_len_with_checksum, &mut out[16..])?;
    let body_offset = 16 + written;
    // 3) Body bytes (Tag 2 onward in ascending order)
    out[body_offset..body_offset + body.len()].copy_from_slice(&body);
    // 4) Tag 1 (checksum) tag + length
    let cksum_tag_offset = body_offset + body.len();
    out[cksum_tag_offset] = 0x01; // tag 1
    out[cksum_tag_offset + 1] = 0x02; // length 2
    // 5) Compute checksum across [UL .. start of checksum value] per
    //    ST 0903.6 §10.1.1. Same running-sum algorithm as ST 0601 §6.3.
    let cksum_value_offset = cksum_tag_offset + 2;
    let cksum = checksum_running_sum_16(&out[..cksum_value_offset]);
    out[cksum_value_offset] = (cksum >> 8) as u8;
    out[cksum_value_offset + 1] = cksum as u8;
    Ok(total)
}

/// Encode a VMTI Local Set as a standalone-VMTI wire record into a
/// fresh `Vec<u8>`. Convenience over [`encode_standalone`] when the
/// caller has no pre-sized buffer.
///
/// # Errors
/// Returns the same [`KlvEncodeError`] variants as [`encode_standalone`].
/// (`KlvEncodeError::BufferTooSmall` cannot fire on this path — the
/// buffer is pre-sized via [`encoded_len_standalone`].)
pub fn encode_to_vec_standalone(ls: &VmtiLs) -> Result<Vec<u8>, KlvEncodeError> {
    let n = encoded_len_standalone(ls);
    let mut buf = vec![0u8; n];
    let written = encode_standalone(ls, &mut buf)?;
    buf.truncate(written);
    Ok(buf)
}

/// Number of wire bytes that [`encode_standalone`] would produce for
/// `ls` — body + 16 (UL) + outer BER length + 4 (Tag 1 TLV).
pub fn encoded_len_standalone(ls: &VmtiLs) -> usize {
    use crate::klv::length::ber_len;
    let body_len = encoded_len(ls);
    // Tag 1 TLV is always exactly 4 bytes (tag + len + 2-byte value).
    let body_len_with_checksum = body_len + 4;
    16 + ber_len(body_len_with_checksum) + body_len_with_checksum
}

/// Number of wire bytes that [`encode`] would produce for `ls`. Mirrors
/// `encode`'s field-by-field structure so the two cannot drift.
pub fn encoded_len(ls: &VmtiLs) -> usize {
    use crate::klv::length::{ber_len, ber_oid_len};
    use crate::klv::st0903::var_uint::var_u32_len;

    fn tlv_len(value_len: usize) -> usize {
        1 /* tag */ + ber_len(value_len) + value_len
    }

    let mut total = 0usize;
    // Tag 1 (checkSum) is omitted by `encode` per ST 0903.6-120; no
    // size contribution.
    if ls.precision_time_stamp.is_some() {
        total += tlv_len(8);
    }
    if let Some(ref s) = ls.vmti_system_name {
        total += tlv_len(s.len());
    }
    if let Some(v) = ls.version_number {
        total += tlv_len(var_u32_len(v as u32));
    }
    if let Some(v) = ls.total_targets_in_frame {
        total += tlv_len(var_u32_len(v));
    }
    if let Some(v) = ls.num_targets_reported {
        total += tlv_len(var_u32_len(v));
    }
    if let Some(v) = ls.frame_width {
        total += tlv_len(var_u32_len(v));
    }
    if let Some(v) = ls.frame_height {
        total += tlv_len(var_u32_len(v));
    }
    if let Some(ref s) = ls.source_sensor {
        total += tlv_len(s.len());
    }
    if ls.horizontal_fov.is_some() {
        total += tlv_len(2);
    }
    if ls.vertical_fov.is_some() {
        total += tlv_len(2);
    }
    if let Some(ref bytes) = ls.miis_id {
        total += tlv_len(bytes.len());
    }
    if !ls.targets.is_empty() {
        let mut series_len = 0usize;
        for pack in &ls.targets {
            let pack_len = vtarget_pack::encoded_len(pack);
            series_len += ber_len(pack_len) + pack_len;
        }
        total += tlv_len(series_len);
    }
    if let Some(ref bytes) = ls.algorithm_series {
        total += tlv_len(bytes.len());
    }
    if let Some(ref bytes) = ls.ontology_series {
        total += tlv_len(bytes.len());
    }
    // Unknown tags use BER-OID tag + BER length + value (mirrors the
    // `write_ber_oid` emit in `encode`). For tags ≤ 127 (the §10.1
    // typed universe), `ber_oid_len(tag) == 1` so this collapses to
    // the same byte count as the pre-E5 `tlv_len(value.len())`.
    for field in &ls.unknown {
        total += ber_oid_len(field.tag) + ber_len(field.value.len()) + field.value.len();
    }
    total
}
