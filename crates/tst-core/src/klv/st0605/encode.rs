//! ST 0605 encode — serialize a Precision Time Stamp Pack to bytes.

use crate::klv::st0605::model::PrecisionTimeStampPack;
use crate::klv::universal_label::UniversalLabel;

/// Encode a Precision Time Stamp Pack to a 26-byte buffer:
/// `[UL:16][BER 0x09:1][status:1][microseconds:8 BE]`.
pub fn encode(pack: &PrecisionTimeStampPack) -> [u8; 26] {
    let mut out = [0u8; 26];
    out[..16].copy_from_slice(&UniversalLabel::PRECISION_TIMESTAMP_PACK_UL.0);
    out[16] = 0x09;
    out[17] = pack.time_status.0;
    out[18..26].copy_from_slice(&pack.timestamp_us.to_be_bytes());
    out
}
