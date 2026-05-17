//! `repr(C)` mirrors of the application-level stats types in
//! `tst-core` / `tst-pipeline`. Layouts match the public C ABI declared in
//! `crates/tst-c/include/tstrans.h` and exercised by `tests/smoke.c`.
//!
//! All wrapping structs use a fixed-size
//! `[TstStreamStats; TST_STATS_MAX_STREAMS]` array +
//! `per_stream_count` + `per_stream_truncated` flag rather than a
//! heap-allocated list. That keeps the ABI allocator-free; callers
//! stack-allocate the stats struct and pass a pointer.

use libc::c_char;

/// Maximum number of per-stream entries in any stats struct exposed at
/// the C ABI. Rarely exceeded — multi-stream `mpegts::mux` caps at
/// 16 video + 16 KLV = 32, leaving 32 slots of headroom for receiver
/// PSI + observed streams.
pub const TST_STATS_MAX_STREAMS: usize = 64;

/// `repr(C)` mirror of `tst_core::mpegts::StreamStats`. Size 96 B.
///
/// Layout (offsets):
///   0: items (u64, 8 B)
///   8: bytes (u64, 8 B)
///  16: discontinuities (u64, 8 B)
///  24: pid (u16, 2 B)
///  26: stream_type (u8, 1 B)
///  27: _pad (3 B, alignment bridge)
///  30: program_number (u16, 2 B)
///  32: label ([c_char; 64], 64 B)
/// Total: 96 B — identical to the pre-program_number layout.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct TstStreamStats {
    pub items: u64,
    pub bytes: u64,
    pub discontinuities: u64,
    pub pid: u16,
    pub stream_type: u8,
    /// Alignment padding bridging stream_type → program_number.
    pub _pad: [u8; 3],
    /// Program number from the PAT that owns this stream. 0 for PSI PIDs.
    pub program_number: u16,
    /// NUL-terminated UTF-8. `label[0]==0` means None. Truncated at 63
    /// bytes (first 63 + NUL).
    pub label: [c_char; 64],
}

const _TST_STREAM_STATS_SIZE: () = assert!(
    std::mem::size_of::<TstStreamStats>() == 96,
    "TstStreamStats must be 96 bytes"
);

impl Default for TstStreamStats {
    fn default() -> Self {
        Self {
            items: 0,
            bytes: 0,
            discontinuities: 0,
            pid: 0,
            stream_type: 0,
            _pad: [0; 3],
            program_number: 0,
            label: [0; 64],
        }
    }
}

/// Copy `src` into the C struct, truncating label to 63 bytes + NUL.
pub fn fill_stream_stats(dst: &mut TstStreamStats, src: &tst_core::mpegts::StreamStats) {
    dst.items = src.items;
    dst.bytes = src.bytes;
    dst.discontinuities = src.discontinuities;
    dst.pid = src.pid;
    dst.stream_type = src.stream_type;
    dst._pad = [0; 3];
    dst.program_number = src.program_number;
    dst.label = [0; 64];
    if let Some(s) = &src.label {
        let bytes = s.as_bytes();
        let n = bytes.len().min(63);
        for (i, b) in bytes[..n].iter().enumerate() {
            dst.label[i] = *b as c_char;
        }
        dst.label[n] = 0;
    }
}

/// `repr(C)` mirror of `tst_pipeline::RawSenderStats`. Size 16 B.
#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct TstRawSenderStats {
    pub bytes_sent: u64,
    pub packets_sent: u64,
}

impl From<&tst_pipeline::RawSenderStats> for TstRawSenderStats {
    fn from(s: &tst_pipeline::RawSenderStats) -> Self {
        Self {
            bytes_sent: s.bytes_sent,
            packets_sent: s.packets_sent,
        }
    }
}

/// `repr(C)` mirror of `tst_pipeline::RawReceiverStats`. Size 16 B.
#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct TstRawReceiverStats {
    pub bytes_received: u64,
    pub packets_received: u64,
}

impl From<&tst_pipeline::RawReceiverStats> for TstRawReceiverStats {
    fn from(s: &tst_pipeline::RawReceiverStats) -> Self {
        Self {
            bytes_received: s.bytes_received,
            packets_received: s.packets_received,
        }
    }
}

/// `repr(C)` mirror of `tst_pipeline::ReceiverStats`. Size 32 B.
///
/// Application-level counters for the TS-aligned receive shell. Mirrors
/// the layout of `tst_pipeline::ReceiverStats`:
/// * `bytes_received` / `packets_received` — application-level totals
///   (one 188-byte packet per `_recv_packet` success).
/// * `bytes_skipped_for_sync` / `resync_events` — sync-recovery state
///   from the underlying `tst_pipeline::receiver::sync::Syncer`. Non-zero
///   `resync_events` indicates the syncer hit HUNT/VERIFY mid-stream
///   (either initial lock-on or re-lock after corruption).
#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct TstReceiverStats {
    pub bytes_received: u64,
    pub bytes_skipped_for_sync: u64,
    pub resync_events: u64,
    pub packets_received: u64,
}

impl From<&tst_pipeline::ReceiverStats> for TstReceiverStats {
    fn from(s: &tst_pipeline::ReceiverStats) -> Self {
        Self {
            bytes_received: s.bytes_received,
            bytes_skipped_for_sync: s.bytes_skipped_for_sync,
            resync_events: s.resync_events,
            packets_received: s.packets_received,
        }
    }
}

/// `repr(C)` mirror of `tst_pipeline::DemuxReceiverStats`. Size 48 B.
///
/// Application-level counters for the demux receive shell. Faithfully
/// mirrors `tst_pipeline::DemuxReceiverStats` — six u64 fields:
/// * `bytes_received` / `packets_received` — application-level totals
///   from the inner `Receiver` (one 188-byte packet per success).
/// * `program_maps_seen` / `pmt_versions_seen` — PSI topology counters.
///   `program_maps_seen` increments on every PMT emission; `pmt_versions_seen`
///   only on a `version_number` bump (PMT churn detector).
/// * `discontinuities` — sum across all PIDs of `DemuxEvent::Discontinuity`
///   emissions (continuity-counter jumps, PES oversize, etc).
/// * `nonconformant` — sum across all PIDs of `DemuxEvent::NonConformant`
///   emissions (17 issue variants; see `tst_event_t.u.nonconformant.issue_code`).
///
/// NOTE: sync-recovery counters (`bytes_skipped_for_sync`, `resync_events`)
/// are deliberately absent — they live only on the inner `Receiver`'s
/// `ReceiverStats` (surfaced via Phase 2's `TstReceiverStats`). Adding
/// them here would mis-label the data source. Consumers needing them
/// run a `tst_receiver_t` instead of a `tst_demux_receiver_t`.
///
/// The per-PID `BTreeMap<u16, StreamStats>` from `DemuxReceiverStats`
/// is NOT included on this struct; it ships separately via
/// `tst_demux_receiver_get_stream_stats` returning a borrowed
/// `(*const TstStreamStats, size_t)` pair per design §4.5.
#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct TstDemuxReceiverStats {
    pub bytes_received: u64,
    pub packets_received: u64,
    pub program_maps_seen: u64,
    pub pmt_versions_seen: u64,
    pub discontinuities: u64,
    pub nonconformant: u64,
}

const _TST_DEMUX_RECEIVER_STATS_SIZE: () = assert!(
    std::mem::size_of::<TstDemuxReceiverStats>() == 48,
    "TstDemuxReceiverStats must be 48 bytes (6 × u64)"
);

impl From<&tst_pipeline::DemuxReceiverStats> for TstDemuxReceiverStats {
    fn from(s: &tst_pipeline::DemuxReceiverStats) -> Self {
        Self {
            bytes_received: s.bytes_received,
            packets_received: s.packets_received,
            program_maps_seen: s.program_maps_seen,
            pmt_versions_seen: s.pmt_versions_seen,
            discontinuities: s.discontinuities,
            nonconformant: s.nonconformant,
        }
    }
}

/// `repr(C)` mirror of `tst_core::transport::SocketStats`. Size 120 B.
///
/// Layout (offsets in bytes — verified by the `_TST_SOCKET_STATS_SIZE`
/// const assertion below):
///   0: rtt_us              (u32, 4 B)
///   4: send_buffer_packets (u32, 4 B)
///   8: recv_buffer_packets (u32, 4 B)
///  12: _pad                (u32, 4 B, alignment bridge to u64 below)
///  16: send_bandwidth_bps  (u64, 8 B)
///  24: recv_bandwidth_bps  (u64, 8 B)
///  32: link_bandwidth_bps  (u64, 8 B)
///  40: bytes_sent          (u64, 8 B)
///  48: packets_sent        (u64, 8 B)
///  56: bytes_received      (u64, 8 B)
///  64: packets_received    (u64, 8 B)
///  72: bytes_lost_recv     (u64, 8 B)
///  80: packets_lost_recv   (u64, 8 B)
///  88: packets_lost_send   (u64, 8 B)
///  96: packets_retransmitted (u64, 8 B)
/// 104: packets_dropped_send  (u64, 8 B)
/// 112: packets_dropped_recv  (u64, 8 B)
/// Total: 120 B.
///
/// All bandwidth fields are bits per second; RTT is microseconds;
/// buffer-depth fields are in packets. See
/// `tst_core::transport::SocketStats` rustdoc for the libsrt source
/// mappings.
#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct TstSocketStats {
    pub rtt_us: u32,
    pub send_buffer_packets: u32,
    pub recv_buffer_packets: u32,
    /// Alignment padding bridging the u32 prefix to the u64 tail.
    pub _pad: u32,
    pub send_bandwidth_bps: u64,
    pub recv_bandwidth_bps: u64,
    pub link_bandwidth_bps: u64,
    pub bytes_sent: u64,
    pub packets_sent: u64,
    pub bytes_received: u64,
    pub packets_received: u64,
    pub bytes_lost_recv: u64,
    pub packets_lost_recv: u64,
    pub packets_lost_send: u64,
    pub packets_retransmitted: u64,
    pub packets_dropped_send: u64,
    pub packets_dropped_recv: u64,
}

const _TST_SOCKET_STATS_SIZE: () = assert!(
    std::mem::size_of::<TstSocketStats>() == 120,
    "TstSocketStats must be 120 bytes (3×u32 + 1×u32 pad + 13×u64)"
);

impl From<&tst_core::transport::SocketStats> for TstSocketStats {
    fn from(s: &tst_core::transport::SocketStats) -> Self {
        Self {
            rtt_us: s.rtt_us,
            send_buffer_packets: s.send_buffer_packets,
            recv_buffer_packets: s.recv_buffer_packets,
            _pad: 0,
            send_bandwidth_bps: s.send_bandwidth_bps,
            recv_bandwidth_bps: s.recv_bandwidth_bps,
            link_bandwidth_bps: s.link_bandwidth_bps,
            bytes_sent: s.bytes_sent,
            packets_sent: s.packets_sent,
            bytes_received: s.bytes_received,
            packets_received: s.packets_received,
            bytes_lost_recv: s.bytes_lost_recv,
            packets_lost_recv: s.packets_lost_recv,
            packets_lost_send: s.packets_lost_send,
            packets_retransmitted: s.packets_retransmitted,
            packets_dropped_send: s.packets_dropped_send,
            packets_dropped_recv: s.packets_dropped_recv,
        }
    }
}

/// `repr(C)` mirror of `tst_core::mpegts::mux::MuxerStats`. Size 6172 B.
#[repr(C)]
pub struct TstMuxerStats {
    pub ts_packets_emitted: u64,
    pub ts_bytes_emitted: u64,
    /// Number of programs (PAT entries) in this muxer's configuration.
    pub programs_configured: u32,
    pub per_stream_count: u32,
    pub per_stream_truncated: u32,
    pub per_stream: [TstStreamStats; TST_STATS_MAX_STREAMS],
}

impl Default for TstMuxerStats {
    fn default() -> Self {
        Self {
            ts_packets_emitted: 0,
            ts_bytes_emitted: 0,
            programs_configured: 0,
            per_stream_count: 0,
            per_stream_truncated: 0,
            per_stream: [TstStreamStats::default(); TST_STATS_MAX_STREAMS],
        }
    }
}

/// `repr(C)` mirror of `tst_pipeline::MuxSenderStats`. Size 6188 B.
#[repr(C)]
pub struct TstMuxSenderStats {
    pub bytes_sent: u64,
    pub packets_sent: u64,
    pub pending_bytes_queued: u64,
    pub pending_chunks_queued: u64,
    /// Number of programs (PAT entries) in the muxer configuration.
    pub programs_configured: u32,
    pub per_stream_count: u32,
    pub per_stream_truncated: u32,
    pub per_stream: [TstStreamStats; TST_STATS_MAX_STREAMS],
}

impl Default for TstMuxSenderStats {
    fn default() -> Self {
        Self {
            bytes_sent: 0,
            packets_sent: 0,
            pending_bytes_queued: 0,
            pending_chunks_queued: 0,
            programs_configured: 0,
            per_stream_count: 0,
            per_stream_truncated: 0,
            per_stream: [TstStreamStats::default(); TST_STATS_MAX_STREAMS],
        }
    }
}

/// Fill a fixed-size C per-stream array from a `BTreeMap<u16, StreamStats>`.
/// Returns `(count, truncated)`. Sorted by PID (BTreeMap iteration order).
pub fn fill_per_stream(
    dst: &mut [TstStreamStats; TST_STATS_MAX_STREAMS],
    src: &std::collections::BTreeMap<u16, tst_core::mpegts::StreamStats>,
) -> (u32, bool) {
    let total = src.len();
    let n = total.min(TST_STATS_MAX_STREAMS);
    for (i, (_pid, ss)) in src.iter().take(n).enumerate() {
        fill_stream_stats(&mut dst[i], ss);
    }
    (n as u32, total > TST_STATS_MAX_STREAMS)
}

/// Tagged-union mirror of [`tst_core::mpegts::stats::StreamCodecStats`].
/// Layout: 4 (kind) + 4 (pad) + 16 (max union arm) = 24 B.
#[repr(C)]
#[derive(Copy, Clone)]
pub struct TstStreamCodecStats {
    /// Discriminator: 0=unknown, 1=video, 2=klv, 3=audio. Additive — new
    /// kinds get new non-zero values; consumers MUST treat unrecognized
    /// kinds as Unknown and ignore `u`.
    pub kind: u32,
    /// Alignment bridge so `u.video` (which starts with a `u64`) is
    /// 8-byte aligned.
    pub _pad: u32,
    pub u: TstStreamCodecStatsUnion,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub union TstStreamCodecStatsUnion {
    pub unknown: TstCodecStatsUnknown,
    pub video: TstCodecStatsVideo,
    pub klv: TstCodecStatsKlv,
    pub audio: TstCodecStatsAudio,
}

#[repr(C)]
#[derive(Copy, Clone, Default)]
pub struct TstCodecStatsUnknown {}

#[repr(C)]
#[derive(Copy, Clone, Default)]
pub struct TstCodecStatsVideo {
    pub nals_or_obus: u64,
    pub random_access_aus: u64,
}

#[repr(C)]
#[derive(Copy, Clone, Default)]
pub struct TstCodecStatsKlv {
    pub records: u64,
}

#[repr(C)]
#[derive(Copy, Clone, Default)]
pub struct TstCodecStatsAudio {
    pub frames: u64,
}

/// Discriminator constants exported as named C constants.
pub const TST_CODEC_KIND_UNKNOWN: u32 = 0;
pub const TST_CODEC_KIND_VIDEO: u32 = 1;
pub const TST_CODEC_KIND_KLV: u32 = 2;
pub const TST_CODEC_KIND_AUDIO: u32 = 3;

/// Convert a Rust public [`tst_core::mpegts::stats::StreamCodecStats`] to
/// the C tagged union.
pub(crate) fn codec_stats_to_c(
    stats: tst_core::mpegts::stats::StreamCodecStats,
) -> TstStreamCodecStats {
    use tst_core::mpegts::stats::StreamCodecStats;
    match stats {
        StreamCodecStats::Unknown => TstStreamCodecStats {
            kind: TST_CODEC_KIND_UNKNOWN,
            _pad: 0,
            u: TstStreamCodecStatsUnion {
                unknown: TstCodecStatsUnknown {},
            },
        },
        StreamCodecStats::Video {
            nals_or_obus,
            random_access_aus,
            ..
        } => TstStreamCodecStats {
            kind: TST_CODEC_KIND_VIDEO,
            _pad: 0,
            u: TstStreamCodecStatsUnion {
                video: TstCodecStatsVideo {
                    nals_or_obus,
                    random_access_aus,
                },
            },
        },
        StreamCodecStats::Klv { records, .. } => TstStreamCodecStats {
            kind: TST_CODEC_KIND_KLV,
            _pad: 0,
            u: TstStreamCodecStatsUnion {
                klv: TstCodecStatsKlv { records },
            },
        },
        StreamCodecStats::Audio { frames, .. } => TstStreamCodecStats {
            kind: TST_CODEC_KIND_AUDIO,
            _pad: 0,
            u: TstStreamCodecStatsUnion {
                audio: TstCodecStatsAudio { frames },
            },
        },
        // #[non_exhaustive] — additive variants surface as Unknown to
        // older callers until they regenerate against a newer header.
        _ => TstStreamCodecStats {
            kind: TST_CODEC_KIND_UNKNOWN,
            _pad: 0,
            u: TstStreamCodecStatsUnion {
                unknown: TstCodecStatsUnknown {},
            },
        },
    }
}

#[cfg(test)]
mod codec_stats_tests {
    use super::*;
    use tst_core::mpegts::stats::StreamCodecStats;

    #[test]
    fn c_struct_size_is_24() {
        assert_eq!(std::mem::size_of::<TstStreamCodecStats>(), 24);
    }

    #[test]
    fn maps_unknown_variant() {
        let c = codec_stats_to_c(StreamCodecStats::Unknown);
        assert_eq!(c.kind, TST_CODEC_KIND_UNKNOWN);
    }

    // NOTE: per-variant unit tests for Video/Klv/Audio mappings are
    // deferred to the Task 12 integration tests that exercise the
    // C entry points against a real Muxer/Demuxer. The variants
    // themselves are `#[non_exhaustive]` (not just the enum), so they
    // cannot be constructed via struct expression from outside tst-core
    // (Rust E0639) — see memory note
    // `reference_non_exhaustive_outside_crate_construction.md`. The
    // discriminator + size invariants are covered above; the field
    // assignments in `codec_stats_to_c` are simple plumbing exercised
    // end-to-end by the Task 12 / Task 13 tests.
}
