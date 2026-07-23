//! MISB ST 0805.1 KLV → Cursor-on-Target (CoT) conversion.
//!
//! ST 0805.1 defines two one-way conversions from a decoded ST 0601 UAS
//! Datalink LS record ([`UasDatalinkLs`]) to Cursor-on-Target XML: a
//! **Platform Position** event (the platform's own position) and a
//! **Sensor Point of Interest** (SPI) event (the sensor's ground aimpoint),
//! linked back to the platform event via `detail/link`.
//!
//! Per §1, the spec deliberately forbids UUIDs for `uid` — CoT generated
//! from a replayed file must be byte-identical to CoT generated live, and a
//! UUID minted after the fact would differ run to run. `uid` is instead a
//! deterministic concatenation of KLV tags: [`platform_uid`] =
//! `"{tag10}_{tag3}"`, [`spi_uid`] = `"{tag10}_{tag3}_{tag11}"`.
//!
//! This module implements KLV→CoT only — ST 0805.1 defines no reverse
//! (CoT→KLV) mapping (see `docs/deferred-features.md`).
//!
//! This is a pure conversion over an already-decoded record, not a KLV byte
//! parser, so it has no fuzz target.

use super::st0601::UasDatalinkLs;
use crate::error::CotError;
use alloc::format;
use alloc::string::String;

/// Configuration for KLV→CoT conversion (ST 0805.1).
#[derive(Debug, Clone, PartialEq)]
pub struct CotConfig {
    /// CoT type for the Platform Position event. ST 0805.1 §5 gives
    /// `a-f-A-M-F` as the fixed-wing example and explicitly requires it be
    /// overridable per platform (rotary-wing, manned pods, ...).
    pub platform_type: String,
    /// `stale = time + update_interval_us`. ST 0805.1 defines `stale` as
    /// "time of next message" but gives no concrete interval — this default
    /// is an implementation choice, not a spec value.
    pub update_interval_us: u64,
    /// XML attribute name stamped in `<detail><_flow-tags_ .../>`.
    pub producer: String,
    /// Geoid undulation (HAE − MSL) applied when only an MSL-referenced
    /// altitude tag is available. `None` emits the MSL value as-is.
    pub geoid_undulation_m: Option<f64>,
    /// CoT `how` attribute. ST 0805.1 §5 fixes this at `m-p`
    /// (machine-passed) for both event types.
    pub how: String,
}

impl Default for CotConfig {
    fn default() -> Self {
        Self {
            platform_type: String::from("a-f-A-M-F"),
            update_interval_us: 5_000_000,
            producer: String::from("ST0601CoT"),
            geoid_undulation_m: None,
            how: String::from("m-p"),
        }
    }
}

/// Deterministic Platform Position `uid`: `"{tag10}_{tag3}"` (ST 0805.1 §5
/// Table 1: "concatenate Tags 10 and 3 separated by an underscore").
pub fn platform_uid(ls: &UasDatalinkLs) -> Result<String, CotError> {
    let tag10 = ls
        .platform_designation
        .as_deref()
        .ok_or(CotError::MissingField {
            tag: 10,
            name: "Platform Designation",
        })?;
    let tag3 = ls.mission_id.as_deref().ok_or(CotError::MissingField {
        tag: 3,
        name: "Mission ID",
    })?;
    Ok(format!("{tag10}_{tag3}"))
}

/// Deterministic SPI `uid`: `"{tag10}_{tag3}_{tag11}"` (ST 0805.1 §5
/// Table 2: "concatenate Tags 10, 3, and 11 with an underscore before and
/// after Tag 3").
pub fn spi_uid(ls: &UasDatalinkLs) -> Result<String, CotError> {
    let tag10 = ls
        .platform_designation
        .as_deref()
        .ok_or(CotError::MissingField {
            tag: 10,
            name: "Platform Designation",
        })?;
    let tag3 = ls.mission_id.as_deref().ok_or(CotError::MissingField {
        tag: 3,
        name: "Mission ID",
    })?;
    let tag11 = ls
        .image_source_sensor
        .as_deref()
        .ok_or(CotError::MissingField {
            tag: 11,
            name: "Image Source Sensor",
        })?;
    Ok(format!("{tag10}_{tag3}_{tag11}"))
}

/// Escapes the five predefined XML entities (`& < > " '`) in `s`.
///
/// Not yet called outside `tests` — wired into the XML assembly in Task E2
/// (`platform_position_xml` / `sensor_point_of_interest_xml`).
#[allow(dead_code)]
fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

/// Formats a POSIX-epoch microsecond timestamp (ST 0601 Tag 2 / MISB
/// ST 0603) as ISO 8601 `YYYY-MM-DDThh:mm:ss.ffffffZ`.
///
/// Pure integer math — no `chrono`, no `std::time` — via the standard
/// Howard-Hinnant `civil_from_days` days-since-epoch → (y, m, d) algorithm,
/// so this stays `no_std`+`alloc`-clean.
///
/// Not yet called outside `tests` — wired into the XML assembly in Task E2.
#[allow(dead_code)]
fn iso8601_us(micros: u64) -> String {
    let total_seconds = micros / 1_000_000;
    let frac_micros = micros % 1_000_000;
    let days = (total_seconds / 86_400) as i64;
    let secs_of_day = total_seconds % 86_400;
    let hh = secs_of_day / 3600;
    let mm = (secs_of_day % 3600) / 60;
    let ss = secs_of_day % 60;

    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}.{frac_micros:06}Z")
}

/// Days-since-1970-01-01 → (year, month, day) in the proleptic Gregorian
/// calendar. Howard Hinnant's `civil_from_days`
/// (<http://howardhinnant.github.io/date_algorithms.html>), ported
/// verbatim — the integer-division tricks are load-bearing, do not
/// "simplify" the arithmetic.
///
/// Only called from `iso8601_us` (also `#[allow(dead_code)]` until Task E2).
#[allow(dead_code)]
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m: u32 = if mp < 10 {
        (mp + 3) as u32
    } else {
        (mp - 9) as u32
    }; // [1, 12]
    (y + i64::from(m <= 2), m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso8601_from_micros_matches_spec_example() {
        assert_eq!(
            iso8601_us(798_039_894_000_000),
            "1995-04-16T13:44:54.000000Z"
        );
        assert_eq!(
            iso8601_us(1_529_588_637_122_999),
            "2018-06-21T13:43:57.122999Z"
        ); // Tag 131 example
    }

    #[test]
    #[allow(clippy::field_reassign_with_default)]
    fn uids_are_deterministic_concatenations() {
        let mut ls = UasDatalinkLs::default();
        ls.platform_designation = Some("PRED01".into());
        ls.mission_id = Some("M05".into());
        ls.image_source_sensor = Some("EO".into());
        assert_eq!(platform_uid(&ls).unwrap(), "PRED01_M05");
        assert_eq!(spi_uid(&ls).unwrap(), "PRED01_M05_EO");
        ls.mission_id = None;
        assert!(matches!(
            platform_uid(&ls).unwrap_err(),
            CotError::MissingField { tag: 3, .. }
        ));
    }

    #[test]
    fn xml_escape_covers_five_entities() {
        assert_eq!(
            xml_escape("a&b<c>d\"e'f"),
            "a&amp;b&lt;c&gt;d&quot;e&apos;f"
        );
    }
}
