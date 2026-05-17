//! H.266 / VVC parameter-set parsers.
//!
//! Hand-rolled per ITU-T H.266 V4 (2026-01) — the bitstream spec
//! (sections 7.3 syntax tables and 7.4 semantics). Mirrors the
//! structure of [`crate::codec::h265`]: per-set parsers
//! ([`parse_vps`] / [`parse_sps`] / [`parse_pps`]) plus a
//! collector ([`parse_parameter_sets`]) that walks a `Vec<NalUnit>`
//! partial-success-tolerantly.
//!
//! ## Scope
//!
//! - VPS, SPS, PPS only. APS NALs (types 17/18), Picture Header NALs
//!   (type 19), multi-layer streams (`nuh_layer_id != 0`), and
//!   general_constraint_info parsing are deferred — see
//!   `docs/deferred-features.md`.
//! - The parser bails with `CodecParseError::UnsupportedProfile` on rare
//!   SPS paths (`scaling_list_data_present_flag = 1`,
//!   `num_short_term_ref_pic_sets > 0`) that aren't exercised by
//!   reference encoder defaults. Same conservative stance as H.265.

pub mod pps;
pub mod profile_tier_level;
pub mod sps;
pub mod vps;
pub mod vui;

pub use pps::{H266Pps, parse_pps};
pub use profile_tier_level::H266ProfileTierLevel;
pub use sps::{H266Sps, parse_sps};
pub use vps::{H266Vps, parse_vps};

use crate::codec::CodecParseError;
use crate::mpegts::demux::event::NalUnit;
use std::collections::BTreeMap;

/// Convenience collector — walks `Vec<NalUnit>` and groups recognized
/// VPS / SPS / PPS by id. Partial-success-tolerant: bad NALs emit
/// `tracing::warn!` and are skipped; `Err` only when every parameter-set
/// NAL fails.
pub fn parse_parameter_sets(nals: &[NalUnit]) -> Result<H266ParameterSets, CodecParseError> {
    let mut vpses: BTreeMap<u8, H266Vps> = BTreeMap::new();
    let mut spses: BTreeMap<u8, H266Sps> = BTreeMap::new();
    let mut ppses: BTreeMap<u8, H266Pps> = BTreeMap::new();
    let mut all_failed = true;
    let mut any_seen = false;

    for nal in nals {
        if let NalUnit::H266 {
            nal_type, payload, ..
        } = nal
        {
            // Per H.266 V4 Table 5: VPS_NUT=14, SPS_NUT=15, PPS_NUT=16.
            match nal_type {
                14 => {
                    any_seen = true;
                    match parse_vps(payload) {
                        Ok(v) => {
                            vpses.insert(v.vps_id, v);
                            all_failed = false;
                        }
                        Err(e) => tracing::warn!(target: "tst_core::codec::h266",
                            error = ?e, "skipping malformed VPS"),
                    }
                }
                15 => {
                    any_seen = true;
                    match parse_sps(payload) {
                        Ok(s) => {
                            spses.insert(s.sps_id, s);
                            all_failed = false;
                        }
                        Err(e) => tracing::warn!(target: "tst_core::codec::h266",
                            error = ?e, "skipping malformed SPS"),
                    }
                }
                16 => {
                    any_seen = true;
                    match parse_pps(payload) {
                        Ok(p) => {
                            ppses.insert(p.pps_id, p);
                            all_failed = false;
                        }
                        Err(e) => tracing::warn!(target: "tst_core::codec::h266",
                            error = ?e, "skipping malformed PPS"),
                    }
                }
                _ => {}
            }
        }
    }
    if any_seen && all_failed {
        return Err(CodecParseError::EngineError(
            "all H.266 parameter-set NALs failed to parse".into(),
        ));
    }
    Ok(H266ParameterSets {
        vpses: vpses.into_values().collect(),
        spses: spses.into_values().collect(),
        ppses: ppses.into_values().collect(),
    })
}

#[must_use]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct H266ParameterSets {
    pub vpses: Vec<H266Vps>,
    pub spses: Vec<H266Sps>,
    pub ppses: Vec<H266Pps>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mpegts::demux::event::NalUnit;

    #[test]
    fn parse_parameter_sets_partial_success_one_bad_sps() {
        // Bad SPS (truncated) + good PPS (minimal valid bytes from pps.rs
        // tests). The collector should warn-and-skip the SPS, keep the
        // PPS, and return Ok with vpses/spses empty + ppses=1.
        let bad_sps = NalUnit::H266 {
            nal_type: 15, // SPS_NUT
            layer_id: 0,
            temporal_id_plus1: 1,
            payload: vec![0x00], // truncated — fails on max_sublayers read
        };
        let good_pps = NalUnit::H266 {
            nal_type: 16, // PPS_NUT
            layer_id: 0,
            temporal_id_plus1: 1,
            payload: vec![0x00, 0x20], // minimal valid PPS
        };
        let result = parse_parameter_sets(&[bad_sps, good_pps]);
        let sets = result.expect("should not fail when at least one parses");
        assert_eq!(sets.vpses.len(), 0);
        assert_eq!(sets.spses.len(), 0);
        assert_eq!(sets.ppses.len(), 1);
    }

    #[test]
    fn parse_parameter_sets_all_bad_returns_err() {
        // Single VPS with empty payload — the only parameter-set NAL fails,
        // so the collector returns EngineError per the all_failed branch.
        let bad_vps = NalUnit::H266 {
            nal_type: 14,
            layer_id: 0,
            temporal_id_plus1: 1,
            payload: vec![],
        };
        let result = parse_parameter_sets(&[bad_vps]);
        assert!(result.is_err());
    }

    #[test]
    fn parse_parameter_sets_no_h266_nals_returns_empty_ok() {
        // Empty input is OK — no NALs seen means no failures, so the
        // collector returns an empty H266ParameterSets.
        let sets = parse_parameter_sets(&[]).expect("empty input ok");
        assert!(sets.vpses.is_empty());
        assert!(sets.spses.is_empty());
        assert!(sets.ppses.is_empty());
    }
}
