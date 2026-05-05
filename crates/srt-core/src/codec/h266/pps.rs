//! H.266 PPS parser. Per H.266 V4 §7.3.2.5.

use crate::codec::ParseError;
use crate::codec::h265::bitreader::BitReader;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct H266Pps {
    pub pps_id: u8,
    pub sps_id: u8,
    pub raw_rbsp: Vec<u8>,
}

/// Parse an H.266 PPS RBSP. Per H.266 V4 §7.3.2.5.
///
/// v0 scope extracts only `pps_id` and `sps_id`. All other fields
/// stay in `raw_rbsp` for consumers needing deeper info later.
pub fn parse_pps(rbsp: &[u8]) -> Result<H266Pps, ParseError> {
    let mut br = BitReader::new(rbsp);
    let pps_id = br.read_u(6)? as u8;
    let sps_id = br.read_u(4)? as u8;
    Ok(H266Pps {
        pps_id,
        sps_id,
        raw_rbsp: rbsp.to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal PPS — pps_id=0, sps_id=0, then a trailing bit.
    /// First byte: pps_id(6)=0 | sps_id(4)[upper 2]=0 → 0b0000_0000
    /// Second byte: sps_id(4)[lower 2]=0 | trailing one + zeros → 0b0010_0000
    fn minimal_pps_rbsp() -> Vec<u8> {
        vec![0x00, 0x20]
    }

    #[test]
    fn parse_pps_minimal() {
        let pps = parse_pps(&minimal_pps_rbsp()).expect("minimal PPS should parse");
        assert_eq!(pps.pps_id, 0);
        assert_eq!(pps.sps_id, 0);
    }
}
