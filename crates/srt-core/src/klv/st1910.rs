//! MISB ST 1910 — AU cell wrapper for synchronous KLV streams.
//!
//! AU cell format per ST 1910 §7:
//! - 16-byte AU Cell Universal Label
//! - BER-encoded value length
//! - Value, which itself starts with:
//!   - 16-byte ST 0605 Precision Time Stamp Pack UL
//!   - BER length (= 9)
//!   - 9-byte body (1-byte Time Status + 8-byte microseconds since 1970)
//!   - Wrapped KLV LS payload
//!
//! Wrap and unwrap are paired: wrap produces a fresh `Vec<u8>` ready for
//! `Muxer::push_klv` (when `klv_carries_pts=true`); unwrap recovers the
//! timestamp and inner KLV bytes for receivers / tests.

use crate::error::KlvDecodeError;
use crate::klv::length::{ber_len, read_ber, write_ber};
use crate::klv::st0605::{self, PrecisionTimeStampPack};
use crate::klv::universal_label::UniversalLabel;

/// Universal Label for the ST 1910 AU cell (per MISB ST 1910 §7).
/// 06 0E 2B 34 02 0B 01 01 0E 01 03 05 06 00 00 00
pub const AU_CELL_UL: UniversalLabel = UniversalLabel([
    0x06, 0x0E, 0x2B, 0x34, 0x02, 0x0B, 0x01, 0x01, 0x0E, 0x01, 0x03, 0x05, 0x06, 0x00, 0x00, 0x00,
]);

/// Wrap `payload` (a KLV LS) with an AU cell header carrying `timestamp`.
/// Returns a fresh `Vec<u8>` ready to pass to `Muxer::push_klv`.
pub fn wrap_au_cell(payload: &[u8], timestamp: PrecisionTimeStampPack) -> Vec<u8> {
    // Embedded PTS pack is the full 26-byte form: UL(16) + BER 0x09(1) + body(9).
    let pts_pack = st0605::encode(&timestamp);
    debug_assert_eq!(pts_pack.len(), 26);

    let value_len = pts_pack.len() + payload.len();
    let outer_len_bytes = ber_len(value_len);
    let mut value_len_buf = [0u8; 9]; // 1 + up to 8 long-form bytes
    let written =
        write_ber(value_len, &mut value_len_buf).expect("9-byte buffer fits any usize BER length");
    debug_assert_eq!(written, outer_len_bytes);

    let mut out = Vec::with_capacity(16 + outer_len_bytes + value_len);
    out.extend_from_slice(&AU_CELL_UL.0);
    out.extend_from_slice(&value_len_buf[..written]);
    out.extend_from_slice(&pts_pack);
    out.extend_from_slice(payload);
    out
}

/// Unwrap an AU cell — return a slice into the wrapped KLV payload and the
/// embedded timestamp.
pub fn unwrap_au_cell(buf: &[u8]) -> Result<(&[u8], PrecisionTimeStampPack), KlvDecodeError> {
    // Outer UL.
    if buf.len() < 16 {
        return Err(KlvDecodeError::Truncated {
            offset: 0,
            needed: 16,
            have: buf.len(),
        });
    }
    let ul = UniversalLabel(<[u8; 16]>::try_from(&buf[..16]).unwrap());
    if ul != AU_CELL_UL {
        return Err(KlvDecodeError::UnexpectedUniversalLabel {
            expected: AU_CELL_UL,
            found: ul,
        });
    }

    // Outer length.
    let (value_len, after_outer_len) = read_ber(&buf[16..])?;
    if after_outer_len.len() < value_len {
        return Err(KlvDecodeError::Truncated {
            offset: buf.len() - after_outer_len.len(),
            needed: value_len,
            have: after_outer_len.len(),
        });
    }
    let value = &after_outer_len[..value_len];

    // Embedded PTS pack starts at value[0]; st0605::decode handles UL + BER length + 9-byte body.
    let timestamp = st0605::decode(value)?;

    // Compute payload offset within `value`: 16 (UL) + bytes_for_BER_length + 9 (body).
    if value.len() < 17 {
        // Need at least UL + 1-byte BER length + 0 bytes body.
        return Err(KlvDecodeError::Truncated {
            offset: 16,
            needed: 17,
            have: value.len(),
        });
    }
    let (declared_body_len, after_inner_len) = read_ber(&value[16..])?;
    let inner_len_bytes = value.len() - 16 - after_inner_len.len();
    if declared_body_len != 9 {
        // st0605::decode would have already rejected this; defensive check here too.
        return Err(KlvDecodeError::MalformedLength { offset: 16 });
    }
    let payload_start = 16 + inner_len_bytes + 9;
    if value.len() < payload_start {
        return Err(KlvDecodeError::Truncated {
            offset: 16,
            needed: payload_start,
            have: value.len(),
        });
    }
    let payload = &value[payload_start..];
    Ok((payload, timestamp))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::klv::st0605::TimeStatus;

    fn sample_pack() -> PrecisionTimeStampPack {
        PrecisionTimeStampPack {
            time_status: TimeStatus(0xFF),
            timestamp_us: 1_700_000_000_000_000,
        }
    }

    #[test]
    fn wrap_unwrap_round_trip() {
        let payload = vec![0xAA; 200];
        let pack = sample_pack();
        let wrapped = wrap_au_cell(&payload, pack);
        let (recovered, ts) = unwrap_au_cell(&wrapped).expect("unwrap");
        assert_eq!(recovered, &payload[..]);
        assert_eq!(ts.timestamp_us, pack.timestamp_us);
        assert_eq!(ts.time_status.0, pack.time_status.0);
    }

    #[test]
    fn wrap_payload_zero_length() {
        let pack = sample_pack();
        let wrapped = wrap_au_cell(&[], pack);
        let (recovered, _) = unwrap_au_cell(&wrapped).unwrap();
        assert!(recovered.is_empty());
    }

    #[test]
    fn unwrap_rejects_wrong_ul() {
        let mut bad = wrap_au_cell(&[0xCC; 10], sample_pack());
        bad[0] = 0x00; // corrupt UL
        let res = unwrap_au_cell(&bad);
        assert!(matches!(
            res,
            Err(KlvDecodeError::UnexpectedUniversalLabel { .. })
        ));
    }

    #[test]
    fn unwrap_rejects_truncated_buffer() {
        let wrapped = wrap_au_cell(&[0xCC; 10], sample_pack());
        let res = unwrap_au_cell(&wrapped[..10]);
        assert!(matches!(res, Err(KlvDecodeError::Truncated { .. })));
    }

    #[test]
    fn au_cell_ul_value() {
        // Spec requires this exact UL.
        let expected: [u8; 16] = [
            0x06, 0x0E, 0x2B, 0x34, 0x02, 0x0B, 0x01, 0x01, 0x0E, 0x01, 0x03, 0x05, 0x06, 0x00,
            0x00, 0x00,
        ];
        assert_eq!(AU_CELL_UL.0, expected);
    }
}
