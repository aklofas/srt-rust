//! `tst_muxer_t` — standalone MPEG-TS muxer utility.
//!
//! Wraps `tst_core::mpegts::mux::Muxer`. No transport — push NALs and KLV,
//! pull TS bytes. The handle is internally synchronized; push_video,
//! push_klv, and pull may be called from different threads.

use crate::config::TstMuxConfig;
use crate::error::{TstError, record_mux_error, set_last_error};
use crate::handle::{Handle, TstKlvStreamHandle, TstVideoStreamHandle};
use tst_core::mpegts::mux::{KlvStreamHandle, Muxer, VideoStreamHandle};

pub struct TstMuxer {
    inner: Handle<Muxer>,
}

/// Open a standalone muxer. Builds the config from `cfg` so the caller may
/// free it immediately after this returns. Returns NULL on failure with
/// last-error set.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_muxer_open(cfg: *mut TstMuxConfig) -> *mut TstMuxer {
    crate::panic::ffi_catch(std::ptr::null_mut(), || {
        let Some(cfg) = (unsafe { cfg.as_mut() }) else {
            set_last_error(TstError::InvalidConfig, "null config pointer");
            return std::ptr::null_mut();
        };
        let built = match cfg.build_config() {
            Ok(c) => c,
            Err(e) => {
                record_mux_error(&e);
                return std::ptr::null_mut();
            }
        };
        let muxer = match Muxer::new(built) {
            Ok(m) => m,
            Err(e) => {
                record_mux_error(&e);
                return std::ptr::null_mut();
            }
        };
        Box::into_raw(Box::new(TstMuxer {
            inner: Handle::new(muxer),
        }))
    })
}

/// Push one Annex-B-framed video access unit. Returns 0 on success or a
/// negative TST_E_* code.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_muxer_push_video(
    p: *mut TstMuxer,
    nal: *const u8,
    len: usize,
    pts_90khz: i64,
    key_frame: bool,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null muxer pointer");
        return TstError::InvalidConfig as i32;
    };
    if nal.is_null() && len > 0 {
        set_last_error(TstError::InvalidConfig, "null nal with non-zero len");
        return TstError::InvalidConfig as i32;
    }
    let slice = unsafe { std::slice::from_raw_parts(nal, len) };
    handle
        .inner
        .with_inner_mut(|m| match m.push_video(slice, pts_90khz, key_frame) {
            Ok(()) => 0,
            Err(e) => {
                record_mux_error(&e);
                // Find the matching TST_E_* code via the recorded last-error.
                unsafe { crate::error::tst_get_last_error() }
            }
        })
}

/// Push one pre-built KLV blob.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_muxer_push_klv(
    p: *mut TstMuxer,
    klv: *const u8,
    len: usize,
    pts_90khz: i64,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null muxer pointer");
        return TstError::InvalidConfig as i32;
    };
    if klv.is_null() && len > 0 {
        set_last_error(TstError::InvalidConfig, "null klv with non-zero len");
        return TstError::InvalidConfig as i32;
    }
    let slice = unsafe { std::slice::from_raw_parts(klv, len) };
    handle.inner.with_inner_mut(|m| {
        match m.push_klv(
            slice, pts_90khz,
            // C ABI receiver-surface plan will expose metadata_service_id;
            // today defaults to 0x00 per ST 1402.2 App. B Table 2.
            0x00,
        ) {
            Ok(()) => 0,
            Err(e) => {
                record_mux_error(&e);
                unsafe { crate::error::tst_get_last_error() }
            }
        }
    })
}

/// Push one Annex-B NAL targeting a specific video elementary stream.
///
/// `handle` is obtained from `tst_mux_config_add_video_stream` at config
/// time and is stable across managed-sender reconnects. Out-of-range
/// handles surface as `TST_E_INVALID_USAGE` (carrying
/// `MuxError::InvalidStreamHandle`).
///
/// On a single-stream muxer, prefer `tst_muxer_push_video` — it has the
/// same effect and doesn't require a handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_muxer_push_video_to(
    p: *mut TstMuxer,
    handle: TstVideoStreamHandle,
    nal: *const u8,
    len: usize,
    pts_90khz: i64,
    key_frame: bool,
) -> libc::c_int {
    let Some(h) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null muxer pointer");
        return TstError::InvalidConfig as i32;
    };
    if nal.is_null() && len > 0 {
        set_last_error(TstError::InvalidConfig, "null nal with non-zero len");
        return TstError::InvalidConfig as i32;
    }
    let slice = unsafe { std::slice::from_raw_parts(nal, len) };
    let stream = VideoStreamHandle::from_raw(handle);
    h.inner.with_inner_mut(
        |m| match m.push_video_to(stream, slice, pts_90khz, key_frame) {
            Ok(()) => 0,
            Err(e) => {
                record_mux_error(&e);
                unsafe { crate::error::tst_get_last_error() }
            }
        },
    )
}

/// Push one pre-built KLV blob targeting a specific KLV elementary stream.
///
/// `handle` is obtained from `tst_mux_config_add_klv_stream`. Same
/// semantics as `tst_muxer_push_video_to`.
///
/// For `KlvStreamType::SynchronousMetadata` streams, the muxer auto-wraps
/// the caller's bytes in a `Metadata_AU_cell` header per ITU-T H.222.0
/// V9 § 2.12.4.2 (5 bytes prepended; PTS surfaced in the PES header).
/// For `KlvStreamType::PrivateData` streams, the caller's bytes pass
/// through unchanged.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_muxer_push_klv_to(
    p: *mut TstMuxer,
    handle: TstKlvStreamHandle,
    klv: *const u8,
    len: usize,
    pts_90khz: i64,
) -> libc::c_int {
    let Some(h) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null muxer pointer");
        return TstError::InvalidConfig as i32;
    };
    if klv.is_null() && len > 0 {
        set_last_error(TstError::InvalidConfig, "null klv with non-zero len");
        return TstError::InvalidConfig as i32;
    }
    let slice = unsafe { std::slice::from_raw_parts(klv, len) };
    let stream = KlvStreamHandle::from_raw(handle);
    h.inner.with_inner_mut(|m| {
        match m.push_klv_to(
            stream, slice, pts_90khz,
            // C ABI receiver-surface plan will expose metadata_service_id;
            // today defaults to 0x00 per ST 1402.2 App. B Table 2.
            0x00,
        ) {
            Ok(()) => 0,
            Err(e) => {
                record_mux_error(&e);
                unsafe { crate::error::tst_get_last_error() }
            }
        }
    })
}

/// Drain TS bytes into `out_buf` (capacity `out_cap`). Returns the number
/// of bytes written; 0 means nothing was ready or the buffer was too
/// small for the next chunk. Never sets last-error — 0 is a normal return
/// value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_muxer_pull(
    p: *mut TstMuxer,
    out_buf: *mut u8,
    out_cap: usize,
) -> usize {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        return 0;
    };
    if out_buf.is_null() || out_cap == 0 {
        return 0;
    }
    let slice = unsafe { std::slice::from_raw_parts_mut(out_buf, out_cap) };
    let mut n = 0usize;
    let _rc = handle.inner.with_inner_mut(|m| {
        n = m.pull(slice);
        0
    });
    n
}

/// Snapshot stats for a `tst_muxer_t` into `*out`.
///
/// Returns 0 on success, `TST_E_INVALID_CONFIG` if either pointer is
/// null, or `TST_E_CLOSED` if the muxer has been closed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_muxer_get_stats(
    p: *mut TstMuxer,
    out: *mut crate::stats::TstMuxerStats,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null muxer pointer");
        return TstError::InvalidConfig as i32;
    };
    if out.is_null() {
        set_last_error(TstError::InvalidConfig, "null out pointer");
        return TstError::InvalidConfig as i32;
    }
    handle.inner.with_inner_ref(|m| {
        let stats = m.stats();
        let mut per_stream =
            [crate::stats::TstStreamStats::default(); crate::stats::TST_STATS_MAX_STREAMS];
        let (per_stream_count, truncated) =
            crate::stats::fill_per_stream(&mut per_stream, &stats.per_stream);
        let dst = crate::stats::TstMuxerStats {
            ts_packets_emitted: stats.ts_packets_emitted,
            ts_bytes_emitted: stats.ts_bytes_emitted,
            programs_configured: stats.programs_configured,
            per_stream_count,
            per_stream_truncated: if truncated { 1 } else { 0 },
            per_stream,
        };
        unsafe { *out = dst };
        0
    })
}

/// Reset stats counters for a `tst_muxer_t` to zero.
///
/// Per-stream entries are preserved (identity fields remain); only flow
/// counters (`items`, `bytes`, `discontinuities`) are zeroed.
///
/// Returns 0 on success, `TST_E_INVALID_CONFIG` if the pointer is
/// null, or `TST_E_CLOSED` if the muxer has been closed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_muxer_reset_stats(p: *mut TstMuxer) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null muxer pointer");
        return TstError::InvalidConfig as i32;
    };
    handle.inner.with_inner_mut(|m| {
        m.reset_stats();
        0
    })
}

/// Close and free the muxer. Idempotent — passing NULL is a no-op.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_muxer_close(p: *mut TstMuxer) {
    if p.is_null() {
        return;
    }
    let boxed = unsafe { Box::from_raw(p) };
    boxed.inner.close();
    drop(boxed);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::*;

    #[test]
    fn open_with_default_config_succeeds() {
        unsafe {
            let cfg = tst_mux_config_new();
            let prog = tst_mux_config_add_program(cfg, 1, 0x1000);
            tst_mux_config_add_video_stream(cfg, prog, 0x1011, TstVideoCodec::H264);
            tst_mux_config_add_klv_stream(cfg, prog, 0x1031, TstKlvStreamType::PrivateData, false);
            let m = tst_muxer_open(cfg);
            assert!(!m.is_null());
            tst_muxer_close(m);
            tst_mux_config_free(cfg);
        }
    }

    #[test]
    fn push_then_pull_emits_bytes() {
        unsafe {
            let cfg = tst_mux_config_new();
            let prog = tst_mux_config_add_program(cfg, 1, 0x1000);
            // Single-stream muxer: push_video (no handle) is unambiguous.
            let hv = tst_mux_config_add_video_stream(cfg, prog, 0x1011, TstVideoCodec::H264);
            tst_mux_config_add_klv_stream(cfg, prog, 0x1031, TstKlvStreamType::PrivateData, false);
            let m = tst_muxer_open(cfg);
            tst_mux_config_free(cfg);

            // Annex-B IDR NAL.
            let nal: [u8; 9] = [0, 0, 0, 1, 0x65, 0xAA, 0xAA, 0xAA, 0xAA];
            let rc = tst_muxer_push_video_to(m, hv, nal.as_ptr(), nal.len(), 0, true);
            assert_eq!(rc, 0);

            let mut buf = vec![0u8; 4096];
            let n = tst_muxer_pull(m, buf.as_mut_ptr(), buf.len());
            assert!(n > 0, "expected TS bytes to be pulled");
            assert_eq!(buf[0], 0x47, "first byte of TS packet must be 0x47");

            tst_muxer_close(m);
        }
    }

    #[test]
    fn push_video_with_invalid_nal_returns_invalid_nal() {
        unsafe {
            let cfg = tst_mux_config_new();
            let prog = tst_mux_config_add_program(cfg, 1, 0x1000);
            tst_mux_config_add_video_stream(cfg, prog, 0x1011, TstVideoCodec::H264);
            tst_mux_config_add_klv_stream(cfg, prog, 0x1031, TstKlvStreamType::PrivateData, false);
            let m = tst_muxer_open(cfg);
            tst_mux_config_free(cfg);

            let bad = [0xAB, 0xCD];
            let rc = tst_muxer_push_video(m, bad.as_ptr(), bad.len(), 0, false);
            assert_eq!(rc, TstError::InvalidNal as i32);

            tst_muxer_close(m);
        }
    }

    #[test]
    fn null_pointer_close_is_safe() {
        unsafe { tst_muxer_close(std::ptr::null_mut()) };
    }
}
