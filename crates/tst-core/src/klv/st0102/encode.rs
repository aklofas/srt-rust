//! ST 0102 encode entry points (`encode`, `encode_to_vec`, `encoded_len`).

use crate::error::KlvEncodeError;
use crate::klv::st0102::model::{SecurityLs, encode_utf16_bom};

/// Encode into a caller-provided buffer. Returns the number of bytes
/// written.
///
/// # Errors
/// - [`KlvEncodeError::BufferTooSmall`] if `out.len()` is less than
///   the required encoded length (call [`encoded_len`] first).
/// - [`KlvEncodeError::RecordTooLarge`] if any individual TLV's
///   declared length would overflow BER encoding (in practice,
///   a value > 2^64 bytes — guards against pathological input).
pub fn encode(record: &SecurityLs, out: &mut [u8]) -> Result<usize, KlvEncodeError> {
    use crate::klv::length::write_ber;

    let needed = encoded_len(record);
    if out.len() < needed {
        return Err(KlvEncodeError::BufferTooSmall {
            needed,
            got: out.len(),
        });
    }

    let mut pos = 0usize;
    let emit =
        |out: &mut [u8], pos: &mut usize, tag: u8, value: &[u8]| -> Result<(), KlvEncodeError> {
            out[*pos] = tag;
            *pos += 1;
            let n = write_ber(value.len(), &mut out[*pos..])
                .map_err(|_| KlvEncodeError::RecordTooLarge)?;
            *pos += n;
            out[*pos..*pos + value.len()].copy_from_slice(value);
            *pos += value.len();
            Ok(())
        };

    // Emit tags in numeric order for determinism. The wire spec
    // doesn't mandate ordering for ST 0102 LS — unlike ST 0601's
    // "tag 2 first, tag 1 last" rules — but emitting numeric is
    // friendlier to byte-diff tooling.
    if let Some(v) = record.security_classification {
        emit(out, &mut pos, 1, &[v.to_u8()])?;
    }
    if let Some(v) = record.classifying_country_coding_method {
        emit(out, &mut pos, 2, &[v.to_u8()])?;
    }
    if let Some(s) = record.classifying_country.as_ref() {
        emit(out, &mut pos, 3, s.as_bytes())?;
    }
    if let Some(s) = record.sci_shi_info.as_ref() {
        emit(out, &mut pos, 4, s.as_bytes())?;
    }
    if let Some(s) = record.caveats.as_ref() {
        emit(out, &mut pos, 5, s.as_bytes())?;
    }
    if let Some(s) = record.releasing_instructions.as_ref() {
        emit(out, &mut pos, 6, s.as_bytes())?;
    }
    if let Some(s) = record.classified_by.as_ref() {
        emit(out, &mut pos, 7, s.as_bytes())?;
    }
    if let Some(s) = record.derived_from.as_ref() {
        emit(out, &mut pos, 8, s.as_bytes())?;
    }
    if let Some(s) = record.classification_reason.as_ref() {
        emit(out, &mut pos, 9, s.as_bytes())?;
    }
    if let Some(s) = record.declassification_date.as_ref() {
        emit(out, &mut pos, 10, s.as_bytes())?;
    }
    if let Some(s) = record.classification_marking_system.as_ref() {
        emit(out, &mut pos, 11, s.as_bytes())?;
    }
    if let Some(v) = record.object_country_coding_method {
        emit(out, &mut pos, 12, &[v.to_u8()])?;
    }
    if let Some(s) = record.object_country_codes.as_ref() {
        let utf16 = encode_utf16_bom(s);
        emit(out, &mut pos, 13, &utf16)?;
    }
    if let Some(s) = record.classification_comments.as_ref() {
        emit(out, &mut pos, 14, s.as_bytes())?;
    }
    if let Some(v) = record.version {
        emit(out, &mut pos, 22, &v.to_be_bytes())?;
    }
    if let Some(s) = record
        .classifying_country_coding_method_version_date
        .as_ref()
    {
        emit(out, &mut pos, 23, s.as_bytes())?;
    }
    if let Some(s) = record.object_country_coding_method_version_date.as_ref() {
        emit(out, &mut pos, 24, s.as_bytes())?;
    }

    // Emit unknown tags last to preserve forward-compat. ST 0102 LS
    // uses single-byte BER-OID tags only; multi-byte tags (>127)
    // are silently dropped — `encoded_len` matches this behavior so
    // the buffer-size precheck stays consistent. The decoder still
    // accepts tag > 127 as forward-compat (ST 0107.5 §6), but
    // encoding round-trip drops them.
    for u in record.unknown.iter() {
        let tag_u8 = match u8::try_from(u.tag) {
            Ok(t) if t <= 127 => t,
            _ => continue, // tag > 127: drop (matches encoded_len)
        };
        emit(out, &mut pos, tag_u8, &u.value)?;
    }

    Ok(pos)
}

/// Encode into a fresh `Vec<u8>`. Convenience over [`encode`] when
/// the caller has no pre-sized buffer.
///
/// # Errors
/// Returns the same [`KlvEncodeError`] variants as [`encode`] minus
/// [`KlvEncodeError::BufferTooSmall`] (the buffer is pre-sized via
/// [`encoded_len`]). [`KlvEncodeError::RecordTooLarge`] can still
/// fire for pathological per-TLV lengths.
pub fn encode_to_vec(record: &SecurityLs) -> Result<Vec<u8>, KlvEncodeError> {
    let mut buf = vec![0u8; encoded_len(record)];
    let n = encode(record, &mut buf)?;
    buf.truncate(n);
    Ok(buf)
}

/// Pre-compute the encoded length for a given record.
pub fn encoded_len(record: &SecurityLs) -> usize {
    use crate::klv::length::ber_len;

    let mut total = 0usize;
    let mut add = |value_len: usize| {
        total += 1 /* tag byte */ + ber_len(value_len) + value_len;
    };

    if record.security_classification.is_some() {
        add(1);
    }
    if record.classifying_country_coding_method.is_some() {
        add(1);
    }
    if let Some(s) = record.classifying_country.as_ref() {
        add(s.len());
    }
    if let Some(s) = record.sci_shi_info.as_ref() {
        add(s.len());
    }
    if let Some(s) = record.caveats.as_ref() {
        add(s.len());
    }
    if let Some(s) = record.releasing_instructions.as_ref() {
        add(s.len());
    }
    if let Some(s) = record.classified_by.as_ref() {
        add(s.len());
    }
    if let Some(s) = record.derived_from.as_ref() {
        add(s.len());
    }
    if let Some(s) = record.classification_reason.as_ref() {
        add(s.len());
    }
    if let Some(s) = record.declassification_date.as_ref() {
        add(s.len());
    }
    if let Some(s) = record.classification_marking_system.as_ref() {
        add(s.len());
    }
    if record.object_country_coding_method.is_some() {
        add(1);
    }
    if let Some(s) = record.object_country_codes.as_ref() {
        // 2 bytes BOM + 2 bytes per UTF-16 code unit
        let utf16_bytes = 2 + s.encode_utf16().count() * 2;
        add(utf16_bytes);
    }
    if let Some(s) = record.classification_comments.as_ref() {
        add(s.len());
    }
    if record.version.is_some() {
        add(2);
    }
    if let Some(s) = record
        .classifying_country_coding_method_version_date
        .as_ref()
    {
        add(s.len());
    }
    if let Some(s) = record.object_country_coding_method_version_date.as_ref() {
        add(s.len());
    }

    for u in record.unknown.iter() {
        // Re-emit unknown tags verbatim. ST 0102 LS uses single-byte
        // BER-OID tags (≤ 127); tags above that range are silently
        // dropped on encode (see the `encode` function's u8::try_from
        // branch), so we don't size for them here either. Asymmetric
        // round-trip is acceptable because the spec doesn't define
        // tags above 127 for ST 0102 LS — preserving them on lenient
        // decode is a forward-compat courtesy, not a contract.
        if u.tag <= 127 {
            total += 1 + ber_len(u.value.len()) + u.value.len();
        }
    }

    total
}
