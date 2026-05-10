//! Public types for the `pairing` module.
//!
//! These are the data shapes the caller sees on the `feed`/`flush`
//! boundary. All are flat structs/enums; no methods. Designed to
//! translate cleanly to future C ABI / JNI / UniFFI surfaces (deferred
//! to the receiver-surface plan).

use std::time::Duration;
use tst_core::mpegts::demux::{DemuxEvent, MetadataKind, StreamId, VideoCodec, VideoPayload};

/// Matching mode for [`Pairer::nearest_pts`](super::Pairer::nearest_pts).
/// `last_before_pts` is past-only by definition and ignores this knob.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchMode {
    /// Emit video on arrival; pair against KLV history only. Lowest
    /// latency. KLV that arrives after its matching video frame is
    /// dropped from pairing (surfaces as `UnpairedKlv` on eviction).
    /// Suitable when the consumer cannot tolerate any pairing-induced
    /// delay (live decoder, on-screen geo overlay, etc.).
    Realtime,
    /// Buffer up to `max_video_buffer` video AUs while looking ahead for
    /// a within-tolerance KLV match. Higher latency, more complete
    /// pairing. Buffered video is force-emitted as `UnpairedVideo` when
    /// (a) a later event proves the tolerance window closed, (b) the
    /// buffer fills (FIFO emission of the oldest entry, with best-effort
    /// match against available history first), or (c) `flush()` is
    /// called. Suitable when the consumer values pairing completeness
    /// over latency (post-flight analysis, archival ingest, log-and-
    /// review pipelines that still feed live data).
    ///
    /// Recommended `max_video_buffer`: 60–120 (≈2–4 s @ 30 fps).
    Buffered { max_video_buffer: usize },
}

/// Pairer matching mode. Audit-direction successor to [`MatchMode`].
///
/// Currently introduced additively alongside `MatchMode`; the
/// follow-up task in this phase swaps `Pairer::nearest_pts` for
/// [`Pairer::with_options`](super::Pairer) which consumes a
/// [`PairerOptions`] containing this enum, at which point `MatchMode`
/// goes away. Field-style `Buffered { max_lag }` is unit-explicit
/// (`Duration` instead of bare ticks) and FFI-friendly.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairerMode {
    /// Pair on each `feed()` call; emit immediately. KLV that arrives
    /// after its matching video frame is dropped from pairing
    /// (surfaces as `UnpairedKlv` on eviction).
    Realtime,
    /// Buffer up to `max_lag` of arrival skew before forced emit.
    /// Higher latency, more complete pairing.
    Buffered {
        /// Maximum arrival-skew window to buffer before forced emit.
        max_lag: Duration,
    },
}

/// Options for [`Pairer::with_options`](super::Pairer).
///
/// Replaces the pre-Phase-3 5-positional-arg `Pairer::nearest_pts`
/// constructor. Field-style construction is unit-explicit
/// (`Duration` instead of bare ticks) and FFI-friendly (the 5-arg
/// shape didn't translate cleanly to UniFFI). Currently introduced
/// additively alongside `nearest_pts`; the follow-up task in this
/// phase swaps `nearest_pts` for `with_options`.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct PairerOptions {
    /// Matching mode: realtime (eager) or buffered (lookahead).
    pub mode: PairerMode,
    /// Maximum |video_pts - klv_pts| considered a match.
    /// `Duration::ZERO` means exact-PTS.
    pub tolerance: Duration,
    /// Cap on KLV records buffered awaiting a video match.
    /// `0` = unbounded.
    pub max_buffered_klv: u64,
    /// Cap on video AUs buffered awaiting a KLV match.
    /// `0` = unbounded.
    pub max_buffered_video: u64,
    /// If `true`, treat KLV-without-matching-video as unmatched (default).
    /// If `false`, emit KLV-only events for downstream consumers.
    pub link_klv_to_video: bool,
}

impl Default for PairerOptions {
    fn default() -> Self {
        Self {
            mode: PairerMode::Realtime,
            tolerance: Duration::from_millis(300),
            max_buffered_klv: 32,
            max_buffered_video: 32,
            link_klv_to_video: true,
        }
    }
}

/// One emission from [`Pairer::feed`](super::Pairer::feed) or
/// [`Pairer::flush`](super::Pairer::flush).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairerOutput {
    /// Video paired with KLV per the configured strategy.
    Paired { video: VideoSample, klv: KlvSample },
    /// Video for which no within-window KLV match could be found.
    UnpairedVideo(VideoSample),
    /// KLV that the pairer ingested but never used in a `Paired` output.
    /// Useful for telemetry — quantifies "metadata received but not
    /// consumed."
    UnpairedKlv(KlvSample),
    /// Any `DemuxEvent` not on the configured `video_pid` or `klv_pid`,
    /// or a Sample/Metadata event on a configured PID whose payload
    /// shape didn't match (e.g., audio Sample on `video_pid`). Surfaces
    /// `ProgramMap`, `NonConformant`, `Discontinuity`, samples on other
    /// PIDs, etc. Caller still has full visibility for topology
    /// discovery and diagnostics.
    PassThrough(DemuxEvent),
}

/// Projection of a `DemuxEvent::Sample { payload: SamplePayload::Video,
/// .. }` event for downstream consumption.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoSample {
    pub stream: StreamId,
    pub pts: i64,
    pub dts: Option<i64>,
    pub codec: VideoCodec,
    pub payload: VideoPayload,
}

/// Projection of a `DemuxEvent::Metadata` event for downstream
/// consumption. `payload` is the raw KLV LS bytes — the demuxer has
/// already peeled the 5-byte H.222.0 §2.12.4.2 AU cell header for
/// `KlvSyncAuCell`, so the bytes feed directly to
/// `tst_core::klv::st0601::decode`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KlvSample {
    pub stream: StreamId,
    pub pts: i64,
    pub kind: MetadataKind,
    pub payload: Vec<u8>,
}

/// Counter snapshot for telemetry. Symmetric with the rest of
/// `tst-pipeline` (per plan #16 conventions).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PairerStats {
    pub paired: u64,
    pub unpaired_video: u64,
    pub unpaired_klv: u64,
    pub pass_through: u64,
}
