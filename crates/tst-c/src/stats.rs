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
