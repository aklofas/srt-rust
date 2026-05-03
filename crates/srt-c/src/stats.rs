//! `repr(C)` mirrors of the application-level stats types in
//! `srt-core`. Layouts match the public C ABI declared in
//! `crates/srt-c/include/srtc.h` and exercised by `tests/smoke.c`.
//!
//! All wrapping structs use a fixed-size
//! `[SrtcStreamStats; SRTC_STATS_MAX_STREAMS]` array +
//! `per_stream_count` + `per_stream_truncated` flag rather than a
//! heap-allocated list. That keeps the ABI allocator-free; callers
//! stack-allocate the stats struct and pass a pointer.

use libc::c_char;

/// Maximum number of per-stream entries in any stats struct exposed at
/// the C ABI. Rarely exceeded — multi-stream `mpegts::mux` caps at
/// 16 video + 16 KLV = 32, leaving 32 slots of headroom for receiver
/// PSI + observed streams.
pub const SRTC_STATS_MAX_STREAMS: usize = 64;

/// `repr(C)` mirror of `srt_core::mpegts::StreamStats`. Size 96 B.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SrtcStreamStats {
    pub items: u64,
    pub bytes: u64,
    pub discontinuities: u64,
    pub pid: u16,
    pub stream_type: u8,
    pub _pad: [u8; 5],
    /// NUL-terminated UTF-8. label[0]==0 means None. Truncated at 63
    /// bytes (first 63 + NUL).
    pub label: [c_char; 64],
}

impl Default for SrtcStreamStats {
    fn default() -> Self {
        Self {
            items: 0,
            bytes: 0,
            discontinuities: 0,
            pid: 0,
            stream_type: 0,
            _pad: [0; 5],
            label: [0; 64],
        }
    }
}

/// Copy `src` into the C struct, truncating label to 63 bytes + NUL.
pub fn fill_stream_stats(dst: &mut SrtcStreamStats, src: &srt_core::mpegts::StreamStats) {
    dst.items = src.items;
    dst.bytes = src.bytes;
    dst.discontinuities = src.discontinuities;
    dst.pid = src.pid;
    dst.stream_type = src.stream_type;
    dst._pad = [0; 5];
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

/// `repr(C)` mirror of `srt_core::pipeline::RawSenderStats`. Size 16 B.
#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct SrtcRawSenderStats {
    pub bytes_sent: u64,
    pub packets_sent: u64,
}

impl From<&srt_core::pipeline::RawSenderStats> for SrtcRawSenderStats {
    fn from(s: &srt_core::pipeline::RawSenderStats) -> Self {
        Self {
            bytes_sent: s.bytes_sent,
            packets_sent: s.packets_sent,
        }
    }
}

/// `repr(C)` mirror of `srt_core::mpegts::mux::MuxerStats`. Size 6168 B.
#[repr(C)]
pub struct SrtcMuxerStats {
    pub ts_packets_emitted: u64,
    pub ts_bytes_emitted: u64,
    pub per_stream_count: u32,
    pub per_stream_truncated: u32,
    pub per_stream: [SrtcStreamStats; SRTC_STATS_MAX_STREAMS],
}

impl Default for SrtcMuxerStats {
    fn default() -> Self {
        Self {
            ts_packets_emitted: 0,
            ts_bytes_emitted: 0,
            per_stream_count: 0,
            per_stream_truncated: 0,
            per_stream: [SrtcStreamStats::default(); SRTC_STATS_MAX_STREAMS],
        }
    }
}

/// `repr(C)` mirror of `srt_core::pipeline::SenderStats`. Size 6184 B.
#[repr(C)]
pub struct SrtcSenderStats {
    pub bytes_sent: u64,
    pub packets_sent: u64,
    pub pending_bytes_queued: u64,
    pub pending_chunks_queued: u64,
    pub per_stream_count: u32,
    pub per_stream_truncated: u32,
    pub per_stream: [SrtcStreamStats; SRTC_STATS_MAX_STREAMS],
}

impl Default for SrtcSenderStats {
    fn default() -> Self {
        Self {
            bytes_sent: 0,
            packets_sent: 0,
            pending_bytes_queued: 0,
            pending_chunks_queued: 0,
            per_stream_count: 0,
            per_stream_truncated: 0,
            per_stream: [SrtcStreamStats::default(); SRTC_STATS_MAX_STREAMS],
        }
    }
}

/// Fill a fixed-size C per-stream array from a `BTreeMap<u16, StreamStats>`.
/// Returns `(count, truncated)`. Sorted by PID (BTreeMap iteration order).
pub fn fill_per_stream(
    dst: &mut [SrtcStreamStats; SRTC_STATS_MAX_STREAMS],
    src: &std::collections::BTreeMap<u16, srt_core::mpegts::StreamStats>,
) -> (u32, bool) {
    let total = src.len();
    let n = total.min(SRTC_STATS_MAX_STREAMS);
    for (i, (_pid, ss)) in src.iter().take(n).enumerate() {
        fill_stream_stats(&mut dst[i], ss);
    }
    (n as u32, total > SRTC_STATS_MAX_STREAMS)
}
