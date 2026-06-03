//! Plan A5b Wave C T13 — `HlsMode` enum + `HlsStats` frozen dataclass.
//!
//! `HlsMode` mirrors `tst_tcp::hls::HlsMode` (Live / Event / Vod). It is
//! an int-comparable PyEnum (`#[pyclass(eq, eq_int)]`) so Python callers
//! can do `mode == HlsMode.LIVE` and `IntEnum`-style ordering.
//!
//! `HlsStats` mirrors `tst_tcp::hls::HlsStats` — the richer per-impl
//! snapshot (3 u64 fields). NOTE: this deviates from the plan T13 sketch,
//! which guessed a 4-field shape with `current_segment_age_us` /
//! `last_segment_duration_us`. The real upstream `HlsStats` carries
//! `segments_written` / `bytes_pushed_total` / `open_segment_bytes`. The
//! duration-style fields live on the universal `PublisherStats`
//! (`HlsPublisher.stats()`), not on `HlsStats` (`HlsPublisher.hls_stats()`).

#![allow(unsafe_op_in_unsafe_fn, clippy::useless_conversion)]

use pyo3::prelude::*;

// ---------------------------------------------------------------------------
// HlsMode — PyEnum mirror of tst_tcp::hls::HlsMode
// ---------------------------------------------------------------------------

/// HLS playlist mode (`tstrans.hls.HlsMode`).
///
/// - `LIVE` — rolling-window playlist; old segments evict. No
///   `#EXT-X-ENDLIST` until `finish()`.
/// - `EVENT` — playlist monotone-grows; `#EXT-X-ENDLIST` on `finish()`.
/// - `VOD` — like EVENT but the playlist is written all-at-once at
///   `finish()`.
#[pyclass(name = "HlsMode", module = "tstrans.hls", eq, eq_int, frozen)]
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
// SHOUTY_SNAKE variant names match the tst-py IntEnum-pyclass convention
// (cf. rtp's TransportPref); this allow covers the lint that would demand
// CamelCase `Live`/`Event`/`Vod`.
#[allow(clippy::upper_case_acronyms)]
pub(crate) enum PyHlsMode {
    // SCREAMING_SNAKE variant names so Python sees `HlsMode.LIVE` etc.
    // (matches the tst-py pyclass-enum convention — cf. rtp's TransportPref
    // `AUTO`/`UDP`/`TCP`). The upstream `tst_tcp::hls::HlsMode` uses
    // `Live`/`Event`/`Vod`; the `From` impls bridge the two namings.
    LIVE,
    EVENT,
    VOD,
}

impl From<PyHlsMode> for tst_tcp::hls::HlsMode {
    fn from(m: PyHlsMode) -> Self {
        match m {
            PyHlsMode::LIVE => Self::Live,
            PyHlsMode::EVENT => Self::Event,
            PyHlsMode::VOD => Self::Vod,
        }
    }
}

impl From<tst_tcp::hls::HlsMode> for PyHlsMode {
    fn from(m: tst_tcp::hls::HlsMode) -> Self {
        match m {
            tst_tcp::hls::HlsMode::Live => Self::LIVE,
            tst_tcp::hls::HlsMode::Event => Self::EVENT,
            tst_tcp::hls::HlsMode::Vod => Self::VOD,
        }
    }
}

#[pymethods]
impl PyHlsMode {
    fn __repr__(&self) -> &'static str {
        match self {
            PyHlsMode::LIVE => "HlsMode.LIVE",
            PyHlsMode::EVENT => "HlsMode.EVENT",
            PyHlsMode::VOD => "HlsMode.VOD",
        }
    }
}

// ---------------------------------------------------------------------------
// HlsStats — frozen mirror of tst_tcp::hls::HlsStats
// ---------------------------------------------------------------------------

/// Richer HLS-specific stats snapshot (`tstrans.hls.HlsStats`).
///
/// Mirrors `tst_tcp::hls::HlsStats`. For cross-publisher metrics use
/// `HlsPublisher.stats()` (returns `PublisherStats`) instead.
#[pyclass(name = "HlsStats", module = "tstrans.hls", frozen, get_all)]
#[derive(Clone)]
pub(crate) struct PyHlsStats {
    /// Total completed segments (history + current run).
    pub segments_written: u64,
    /// Total bytes accepted by `push_ts` across all segments.
    pub bytes_pushed_total: u64,
    /// Bytes in the currently-open segment (0 between cuts).
    pub open_segment_bytes: u64,
}

impl From<tst_tcp::hls::HlsStats> for PyHlsStats {
    fn from(s: tst_tcp::hls::HlsStats) -> Self {
        Self {
            segments_written: s.segments_written,
            bytes_pushed_total: s.bytes_pushed_total,
            open_segment_bytes: s.open_segment_bytes,
        }
    }
}

#[pymethods]
impl PyHlsStats {
    #[new]
    fn new(segments_written: u64, bytes_pushed_total: u64, open_segment_bytes: u64) -> Self {
        Self {
            segments_written,
            bytes_pushed_total,
            open_segment_bytes,
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "HlsStats(segments_written={}, bytes_pushed_total={}, open_segment_bytes={})",
            self.segments_written, self.bytes_pushed_total, self.open_segment_bytes,
        )
    }
}
