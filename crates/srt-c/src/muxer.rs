//! `srtc_muxer_t` — standalone MPEG-TS muxer utility.
//!
//! Wraps `srt_core::mpegts::mux::Muxer`. No transport — push NALs and KLV,
//! pull TS bytes. The handle is internally synchronized; push_video,
//! push_klv, and pull may be called from different threads.

use crate::config::SrtcMuxConfig;
use crate::error::{SrtcError, record_mux_error, set_last_error};
use crate::handle::Handle;
use srt_core::mpegts::mux::Muxer;

pub struct SrtcMuxer {
    inner: Handle<Muxer>,
}

/// Open a standalone muxer. Clones the inner of `cfg` so the caller may
/// free it immediately after this returns. Returns NULL on failure with
/// last-error set.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_muxer_open(cfg: *const SrtcMuxConfig) -> *mut SrtcMuxer {
    let Some(cfg) = (unsafe { cfg.as_ref() }) else {
        set_last_error(SrtcError::InvalidConfig, "null config pointer");
        return std::ptr::null_mut();
    };
    let built = match cfg.builder.clone().build() {
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
    Box::into_raw(Box::new(SrtcMuxer {
        inner: Handle::new(muxer),
    }))
}

/// Push one Annex-B-framed video access unit. Returns 0 on success or a
/// negative SRTC_E_* code.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_muxer_push_video(
    p: *mut SrtcMuxer,
    nal: *const u8,
    len: usize,
    pts_90khz: i64,
    key_frame: bool,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(SrtcError::InvalidConfig, "null muxer pointer");
        return SrtcError::InvalidConfig as i32;
    };
    if nal.is_null() && len > 0 {
        set_last_error(SrtcError::InvalidConfig, "null nal with non-zero len");
        return SrtcError::InvalidConfig as i32;
    }
    let slice = unsafe { std::slice::from_raw_parts(nal, len) };
    handle
        .inner
        .with_inner_mut(|m| match m.push_video(slice, pts_90khz, key_frame) {
            Ok(()) => 0,
            Err(e) => {
                record_mux_error(&e);
                // Find the matching SRTC_E_* code via the recorded last-error.
                unsafe { crate::error::srtc_get_last_error() }
            }
        })
}

/// Push one pre-built KLV blob.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_muxer_push_klv(
    p: *mut SrtcMuxer,
    klv: *const u8,
    len: usize,
    pts_90khz: i64,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(SrtcError::InvalidConfig, "null muxer pointer");
        return SrtcError::InvalidConfig as i32;
    };
    if klv.is_null() && len > 0 {
        set_last_error(SrtcError::InvalidConfig, "null klv with non-zero len");
        return SrtcError::InvalidConfig as i32;
    }
    let slice = unsafe { std::slice::from_raw_parts(klv, len) };
    handle
        .inner
        .with_inner_mut(|m| match m.push_klv(slice, pts_90khz) {
            Ok(()) => 0,
            Err(e) => {
                record_mux_error(&e);
                unsafe { crate::error::srtc_get_last_error() }
            }
        })
}

/// Drain TS bytes into `out_buf` (capacity `out_cap`). Returns the number
/// of bytes written; 0 means nothing was ready or the buffer was too
/// small for the next chunk. Never sets last-error — 0 is a normal return
/// value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_muxer_pull(
    p: *mut SrtcMuxer,
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

/// Close and free the muxer. Idempotent — passing NULL is a no-op.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_muxer_close(p: *mut SrtcMuxer) {
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
            let cfg = srtc_mux_config_new();
            srtc_mux_config_add_video(cfg, 0x1011, SrtcVideoCodec::H264);
            srtc_mux_config_add_klv(cfg, 0x1031, SrtcKlvStreamType::PrivateData, false);
            let m = srtc_muxer_open(cfg);
            assert!(!m.is_null());
            srtc_muxer_close(m);
            srtc_mux_config_free(cfg);
        }
    }

    #[test]
    fn push_then_pull_emits_bytes() {
        unsafe {
            let cfg = srtc_mux_config_new();
            srtc_mux_config_add_video(cfg, 0x1011, SrtcVideoCodec::H264);
            srtc_mux_config_add_klv(cfg, 0x1031, SrtcKlvStreamType::PrivateData, false);
            let m = srtc_muxer_open(cfg);
            srtc_mux_config_free(cfg);

            // Annex-B IDR NAL.
            let nal: [u8; 9] = [0, 0, 0, 1, 0x65, 0xAA, 0xAA, 0xAA, 0xAA];
            let rc = srtc_muxer_push_video(m, nal.as_ptr(), nal.len(), 0, true);
            assert_eq!(rc, 0);

            let mut buf = vec![0u8; 4096];
            let n = srtc_muxer_pull(m, buf.as_mut_ptr(), buf.len());
            assert!(n > 0, "expected TS bytes to be pulled");
            assert_eq!(buf[0], 0x47, "first byte of TS packet must be 0x47");

            srtc_muxer_close(m);
        }
    }

    #[test]
    fn push_video_with_invalid_nal_returns_invalid_nal() {
        unsafe {
            let cfg = srtc_mux_config_new();
            srtc_mux_config_add_video(cfg, 0x1011, SrtcVideoCodec::H264);
            srtc_mux_config_add_klv(cfg, 0x1031, SrtcKlvStreamType::PrivateData, false);
            let m = srtc_muxer_open(cfg);
            srtc_mux_config_free(cfg);

            let bad = [0xAB, 0xCD];
            let rc = srtc_muxer_push_video(m, bad.as_ptr(), bad.len(), 0, false);
            assert_eq!(rc, SrtcError::InvalidNal as i32);

            srtc_muxer_close(m);
        }
    }

    #[test]
    fn null_pointer_close_is_safe() {
        unsafe { srtc_muxer_close(std::ptr::null_mut()) };
    }
}
