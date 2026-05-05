//! H.266 VPS parser. Per H.266 V4 §7.3.2.3.

use crate::codec::ParseError;
use crate::codec::h265::bitreader::BitReader; // shared with codec::h266 — codec-agnostic Annex-B bitreader

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct H266Vps {
    pub vps_id: u8,
    pub max_layers: u8,
    pub max_sub_layers: u8,
    pub raw_rbsp: Vec<u8>,
}

/// Parse an H.266 VPS RBSP (Annex-B start codes already stripped,
/// emulation-prevention bytes preserved). Per H.266 V4 §7.3.2.3.
///
/// v0 scope extracts only `vps_id`, `max_layers`, `max_sub_layers`.
/// Profile/Tier/Level loops, OLS info, DPB/HRD parameters are not
/// surfaced — `raw_rbsp` carries the full input so consumers needing
/// more can call deeper parsers later.
pub fn parse_vps(rbsp: &[u8]) -> Result<H266Vps, ParseError> {
    let mut br = BitReader::new(rbsp);
    let vps_id = br.read_u(4)? as u8;
    let max_layers_minus1 = br.read_u(6)? as u8;
    let max_sublayers_minus1 = br.read_u(3)? as u8;
    Ok(H266Vps {
        vps_id,
        max_layers: max_layers_minus1.saturating_add(1),
        max_sub_layers: max_sublayers_minus1.saturating_add(1),
        raw_rbsp: rbsp.to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal VPS RBSP — vps_id=0, max_layers_minus1=0, max_sublayers_minus1=0,
    /// rest zeros. 4+6+3+... = enough bits to exit the v0 parse early.
    /// Constructed by hand: nibbles 0000 (vps_id=0) | 000000 (max_layers=0) |
    /// 000 (max_sublayers=0) | 0 (default_ptl flag=0) | 0 (all_indep=0) |
    /// trailing rbsp_trailing_bits(): 1 bit set + zero pad to byte align.
    fn minimal_vps_rbsp() -> Vec<u8> {
        // First byte:  vps_id(4)=0 | max_layers_minus1(6)[upper 4]=0 → 0b0000_0000
        // Second byte: max_layers_minus1(6)[lower 2]=0 | max_sublayers(3)=0 |
        //              default_ptl_flag=0 | all_indep=0 | trailing one bit=1
        //              → 0b0000_0010
        vec![0x00, 0x02]
    }

    #[test]
    fn parse_vps_minimal() {
        let rbsp = minimal_vps_rbsp();
        let vps = parse_vps(&rbsp).expect("minimal VPS should parse");
        assert_eq!(vps.vps_id, 0);
        assert_eq!(vps.max_layers, 1, "max_layers = max_layers_minus1 + 1");
        assert_eq!(vps.max_sub_layers, 1);
        assert_eq!(vps.raw_rbsp, rbsp);
    }

    #[test]
    fn parse_vps_truncated_returns_err() {
        let truncated = vec![];
        assert!(parse_vps(&truncated).is_err());
    }
}
