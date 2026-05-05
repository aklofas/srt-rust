//! `srtc_mux_sender_t` (plain) and `srtc_managed_mux_sender_t` (managed).
//!
//! Both wrap `srt_core::pipeline::Sender<T>`, with T parameterized on the
//! underlying transport. Plain uses `SrtTransport`; managed uses
//! `ManagedTransport<SrtTransport>` with a factory that reconnects via the
//! original URL on transport breakage.

use crate::config::{SrtcMuxConfig, SrtcReconnectPolicy};
use crate::error::{
    SrtcError, record_mux_error, record_sender_error, set_last_error, srtc_get_last_error,
};
use crate::handle::{Handle, SrtcKlvStreamHandle, SrtcVideoStreamHandle};
use srt_core::mpegts::mux::{KlvStreamHandle, VideoStreamHandle};
use srt_core::pipeline::{ManagedTransport, Sender, SrtTransport};
use srt_core::srt::config::SocketConfig;

// ------------------------------------------------------------------
// srtc_mux_sender_t (plain L1)
// ------------------------------------------------------------------

pub struct SrtcMuxSender {
    inner: Handle<Sender<SrtTransport>>,
}

/// Open a `srtc_mux_sender_t` connected via SRT.
///
/// `srt_url` is a `srt://host:port?key=value&...` URL. Query
/// parameters apply libsrt-vocabulary options to the connection
/// (passphrase, latency, streamid, etc.). URL values override config
/// values for the same option. See
/// `docs/guide-srt.md#url-parsing` for the recognized key table.
///
/// Returns `NULL` with `SRTC_E_INVALID_CONFIG` set in the thread-local
/// last-error for any malformed URL, unsupported key, unknown key, or
/// invalid value. The detail string from
/// `srtc_get_last_error_str()` describes the specific problem.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_mux_sender_open(
    srt_url: *const libc::c_char,
    cfg: *mut SrtcMuxConfig,
) -> *mut SrtcMuxSender {
    let Some(cfg) = (unsafe { cfg.as_mut() }) else {
        set_last_error(SrtcError::InvalidConfig, "null config pointer");
        return std::ptr::null_mut();
    };
    let url = match unsafe { parse_c_srt_url(srt_url) } {
        Ok(u) => u,
        Err(()) => return std::ptr::null_mut(),
    };
    let built = match cfg.build_config() {
        Ok(c) => c,
        Err(e) => {
            record_mux_error(&e);
            return std::ptr::null_mut();
        }
    };
    let mut socket_cfg = SocketConfig::default();
    url.overlay.apply_to_socket(&mut socket_cfg);
    let transport = match crate::connect::connect_srt(&url.host, url.port, &socket_cfg) {
        Ok(t) => t,
        Err(e) => {
            crate::error::record_transport_error(&e);
            return std::ptr::null_mut();
        }
    };
    let sender = match Sender::new(built, transport) {
        Ok(s) => s,
        Err(e) => {
            record_mux_error(&e);
            return std::ptr::null_mut();
        }
    };
    Box::into_raw(Box::new(SrtcMuxSender {
        inner: Handle::new(sender),
    }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_mux_sender_send_video(
    p: *mut SrtcMuxSender,
    nal: *const u8,
    len: usize,
    pts_90khz: i64,
    key_frame: bool,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(SrtcError::InvalidConfig, "null sender pointer");
        return SrtcError::InvalidConfig as i32;
    };
    if nal.is_null() && len > 0 {
        set_last_error(SrtcError::InvalidConfig, "null nal with non-zero len");
        return SrtcError::InvalidConfig as i32;
    }
    let slice = unsafe { std::slice::from_raw_parts(nal, len) };
    handle
        .inner
        .with_inner_ref(|s| match s.send_video(slice, pts_90khz, key_frame) {
            Ok(()) => 0,
            Err(e) => {
                record_sender_error(&e);
                unsafe { srtc_get_last_error() }
            }
        })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_mux_sender_send_klv(
    p: *mut SrtcMuxSender,
    klv: *const u8,
    len: usize,
    pts_90khz: i64,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(SrtcError::InvalidConfig, "null sender pointer");
        return SrtcError::InvalidConfig as i32;
    };
    if klv.is_null() && len > 0 {
        set_last_error(SrtcError::InvalidConfig, "null klv with non-zero len");
        return SrtcError::InvalidConfig as i32;
    }
    let slice = unsafe { std::slice::from_raw_parts(klv, len) };
    handle
        .inner
        .with_inner_ref(|s| match s.send_klv(slice, pts_90khz) {
            Ok(()) => 0,
            Err(e) => {
                record_sender_error(&e);
                unsafe { srtc_get_last_error() }
            }
        })
}

/// Push one Annex-B NAL targeting a specific video elementary stream.
///
/// `stream_handle` is obtained from `srtc_mux_config_add_video_stream` at
/// config time and is stable across the config→open boundary. Out-of-range
/// handles surface as `SRTC_E_INVALID_USAGE` (carrying
/// `MuxError::InvalidStreamHandle`).
///
/// On a single-stream sender, prefer `srtc_mux_sender_send_video` — same
/// effect, no handle required.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_mux_sender_send_video_to(
    p: *mut SrtcMuxSender,
    stream_handle: SrtcVideoStreamHandle,
    nal: *const u8,
    len: usize,
    pts_90khz: i64,
    key_frame: bool,
) -> libc::c_int {
    let Some(wrapper) = (unsafe { p.as_ref() }) else {
        set_last_error(SrtcError::InvalidConfig, "null sender pointer");
        return SrtcError::InvalidConfig as i32;
    };
    if nal.is_null() && len > 0 {
        set_last_error(SrtcError::InvalidConfig, "null nal with non-zero len");
        return SrtcError::InvalidConfig as i32;
    }
    let slice = unsafe { std::slice::from_raw_parts(nal, len) };
    let stream = VideoStreamHandle::from_raw(stream_handle);
    wrapper.inner.with_inner_ref(
        |s| match s.send_video_to(stream, slice, pts_90khz, key_frame) {
            Ok(()) => 0,
            Err(e) => {
                record_sender_error(&e);
                unsafe { srtc_get_last_error() }
            }
        },
    )
}

/// Push one pre-built KLV blob targeting a specific KLV elementary stream.
///
/// For `KlvStreamType::SynchronousMetadata` streams, the muxer auto-wraps
/// the caller's bytes in a `Metadata_AU_cell` header per ITU-T H.222.0
/// V9 § 2.12.4.2 (5 bytes prepended; PTS surfaced in the PES header).
/// For `KlvStreamType::PrivateData` streams, the caller's bytes pass
/// through unchanged.
///
/// On a single-stream sender, prefer `srtc_mux_sender_send_klv` — same
/// effect, no handle required.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_mux_sender_send_klv_to(
    p: *mut SrtcMuxSender,
    stream_handle: SrtcKlvStreamHandle,
    klv: *const u8,
    len: usize,
    pts_90khz: i64,
) -> libc::c_int {
    let Some(wrapper) = (unsafe { p.as_ref() }) else {
        set_last_error(SrtcError::InvalidConfig, "null sender pointer");
        return SrtcError::InvalidConfig as i32;
    };
    if klv.is_null() && len > 0 {
        set_last_error(SrtcError::InvalidConfig, "null klv with non-zero len");
        return SrtcError::InvalidConfig as i32;
    }
    let slice = unsafe { std::slice::from_raw_parts(klv, len) };
    let stream = KlvStreamHandle::from_raw(stream_handle);
    wrapper
        .inner
        .with_inner_ref(|s| match s.send_klv_to(stream, slice, pts_90khz) {
            Ok(()) => 0,
            Err(e) => {
                record_sender_error(&e);
                unsafe { srtc_get_last_error() }
            }
        })
}

/// Snapshot stats for a `srtc_mux_sender_t` into `*out`.
///
/// Returns 0 on success, `SRTC_E_INVALID_CONFIG` if either pointer is
/// null, or `SRTC_E_CLOSED` if the sender has been closed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_mux_sender_get_stats(
    p: *mut SrtcMuxSender,
    out: *mut crate::stats::SrtcSenderStats,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(SrtcError::InvalidConfig, "null sender pointer");
        return SrtcError::InvalidConfig as i32;
    };
    if out.is_null() {
        set_last_error(SrtcError::InvalidConfig, "null out pointer");
        return SrtcError::InvalidConfig as i32;
    }
    handle.inner.with_inner_ref(|s| {
        let stats = s.stats();
        let mut per_stream =
            [crate::stats::SrtcStreamStats::default(); crate::stats::SRTC_STATS_MAX_STREAMS];
        let (per_stream_count, truncated) =
            crate::stats::fill_per_stream(&mut per_stream, &stats.per_stream);
        let dst = crate::stats::SrtcSenderStats {
            bytes_sent: stats.bytes_sent,
            packets_sent: stats.packets_sent,
            pending_bytes_queued: stats.pending_bytes_queued,
            pending_chunks_queued: stats.pending_chunks_queued,
            programs_configured: stats.programs_configured,
            per_stream_count,
            per_stream_truncated: if truncated { 1 } else { 0 },
            per_stream,
        };
        unsafe { *out = dst };
        0
    })
}

/// Reset stats counters for a `srtc_mux_sender_t` to zero.
///
/// Returns 0 on success, `SRTC_E_INVALID_CONFIG` if the pointer is
/// null, or `SRTC_E_CLOSED` if the sender has been closed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_mux_sender_reset_stats(p: *mut SrtcMuxSender) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(SrtcError::InvalidConfig, "null sender pointer");
        return SrtcError::InvalidConfig as i32;
    };
    handle.inner.with_inner_ref(|s| {
        s.reset_stats();
        0
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_mux_sender_close(p: *mut SrtcMuxSender) {
    if p.is_null() {
        return;
    }
    let boxed = unsafe { Box::from_raw(p) };
    boxed.inner.close();
    drop(boxed);
}

/// Borrow `srt_url` as a Rust string and run it through `srt_core`'s
/// rich URL parser. Sets last-error and returns `Err(())` on any failure
/// path; caller treats `Err(())` as "return NULL".
pub(crate) unsafe fn parse_c_srt_url(srt_url: *const libc::c_char) -> Result<srt_core::SrtUrl, ()> {
    if srt_url.is_null() {
        set_last_error(SrtcError::InvalidConfig, "null srt_url");
        return Err(());
    }
    let cstr = unsafe { std::ffi::CStr::from_ptr(srt_url) };
    let s = match cstr.to_str() {
        Ok(s) => s,
        Err(_) => {
            set_last_error(SrtcError::InvalidConfig, "srt_url is not valid utf-8");
            return Err(());
        }
    };
    srt_core::SrtUrl::parse(s).map_err(|e| {
        set_last_error(SrtcError::InvalidConfig, &format!("invalid srt url: {e}"));
    })
}

// ------------------------------------------------------------------
// srtc_managed_mux_sender_t (managed L2)
// ------------------------------------------------------------------

pub struct SrtcManagedMuxSender {
    inner: Handle<Sender<ManagedTransport<SrtTransport>>>,
}

/// Open a `srtc_managed_mux_sender_t` connected via SRT.
///
/// `srt_url` is a `srt://host:port?key=value&...` URL. Query
/// parameters apply libsrt-vocabulary options to the connection
/// (passphrase, latency, streamid, etc.). URL values override config
/// values for the same option. See
/// `docs/guide-srt.md#url-parsing` for the recognized key table.
///
/// Returns `NULL` with `SRTC_E_INVALID_CONFIG` set in the thread-local
/// last-error for any malformed URL, unsupported key, unknown key, or
/// invalid value. The detail string from
/// `srtc_get_last_error_str()` describes the specific problem.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_managed_mux_sender_open(
    srt_url: *const libc::c_char,
    cfg: *mut SrtcMuxConfig,
    policy: *const SrtcReconnectPolicy,
) -> *mut SrtcManagedMuxSender {
    let Some(cfg) = (unsafe { cfg.as_mut() }) else {
        set_last_error(SrtcError::InvalidConfig, "null config pointer");
        return std::ptr::null_mut();
    };
    let policy = match unsafe { policy.as_ref() } {
        Some(p) => p.inner.clone(),
        None => srt_core::pipeline::ReconnectPolicy::default(),
    };
    let url = match unsafe { parse_c_srt_url(srt_url) } {
        Ok(u) => u,
        Err(()) => return std::ptr::null_mut(),
    };
    let built = match cfg.build_config() {
        Ok(c) => c,
        Err(e) => {
            record_mux_error(&e);
            return std::ptr::null_mut();
        }
    };
    let mut socket_cfg = SocketConfig::default();
    url.overlay.apply_to_socket(&mut socket_cfg);

    // Initial connect.
    let initial = match crate::connect::connect_srt(&url.host, url.port, &socket_cfg) {
        Ok(t) => t,
        Err(e) => {
            crate::error::record_transport_error(&e);
            return std::ptr::null_mut();
        }
    };

    // Reconnect closure: same host/port AND same socket config so URL
    // overlay options (passphrase/latency/etc.) survive reconnects.
    // URL is parsed once at construction and never re-parsed.
    let host = url.host.clone();
    let port = url.port;
    let cfg_for_reconnect = socket_cfg.clone();
    let factory = move || crate::connect::connect_srt(&host, port, &cfg_for_reconnect);

    let managed = ManagedTransport::new(initial, factory, policy);
    let sender = match Sender::new(built, managed) {
        Ok(s) => s,
        Err(e) => {
            record_mux_error(&e);
            return std::ptr::null_mut();
        }
    };
    Box::into_raw(Box::new(SrtcManagedMuxSender {
        inner: Handle::new(sender),
    }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_managed_mux_sender_send_video(
    p: *mut SrtcManagedMuxSender,
    nal: *const u8,
    len: usize,
    pts_90khz: i64,
    key_frame: bool,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(SrtcError::InvalidConfig, "null sender pointer");
        return SrtcError::InvalidConfig as i32;
    };
    if nal.is_null() && len > 0 {
        set_last_error(SrtcError::InvalidConfig, "null nal with non-zero len");
        return SrtcError::InvalidConfig as i32;
    }
    let slice = unsafe { std::slice::from_raw_parts(nal, len) };
    handle
        .inner
        .with_inner_ref(|s| match s.send_video(slice, pts_90khz, key_frame) {
            Ok(()) => 0,
            Err(e) => {
                record_sender_error(&e);
                unsafe { srtc_get_last_error() }
            }
        })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_managed_mux_sender_send_klv(
    p: *mut SrtcManagedMuxSender,
    klv: *const u8,
    len: usize,
    pts_90khz: i64,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(SrtcError::InvalidConfig, "null sender pointer");
        return SrtcError::InvalidConfig as i32;
    };
    if klv.is_null() && len > 0 {
        set_last_error(SrtcError::InvalidConfig, "null klv with non-zero len");
        return SrtcError::InvalidConfig as i32;
    }
    let slice = unsafe { std::slice::from_raw_parts(klv, len) };
    handle
        .inner
        .with_inner_ref(|s| match s.send_klv(slice, pts_90khz) {
            Ok(()) => 0,
            Err(e) => {
                record_sender_error(&e);
                unsafe { srtc_get_last_error() }
            }
        })
}

/// Push one Annex-B NAL targeting a specific video elementary stream on a
/// managed (auto-reconnecting) sender.
///
/// `stream_handle` is obtained from `srtc_mux_config_add_video_stream` at
/// config time and is stable across reconnects. Out-of-range handles
/// surface as `SRTC_E_INVALID_USAGE` (carrying
/// `MuxError::InvalidStreamHandle`).
///
/// On a single-stream sender, prefer `srtc_managed_mux_sender_send_video` —
/// same effect, no handle required.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_managed_mux_sender_send_video_to(
    p: *mut SrtcManagedMuxSender,
    stream_handle: SrtcVideoStreamHandle,
    nal: *const u8,
    len: usize,
    pts_90khz: i64,
    key_frame: bool,
) -> libc::c_int {
    let Some(wrapper) = (unsafe { p.as_ref() }) else {
        set_last_error(SrtcError::InvalidConfig, "null sender pointer");
        return SrtcError::InvalidConfig as i32;
    };
    if nal.is_null() && len > 0 {
        set_last_error(SrtcError::InvalidConfig, "null nal with non-zero len");
        return SrtcError::InvalidConfig as i32;
    }
    let slice = unsafe { std::slice::from_raw_parts(nal, len) };
    let stream = VideoStreamHandle::from_raw(stream_handle);
    wrapper.inner.with_inner_ref(
        |s| match s.send_video_to(stream, slice, pts_90khz, key_frame) {
            Ok(()) => 0,
            Err(e) => {
                record_sender_error(&e);
                unsafe { srtc_get_last_error() }
            }
        },
    )
}

/// Push one pre-built KLV blob targeting a specific KLV elementary stream on
/// a managed (auto-reconnecting) sender.
///
/// For `KlvStreamType::SynchronousMetadata` streams, the muxer auto-wraps
/// the caller's bytes in a `Metadata_AU_cell` header per ITU-T H.222.0
/// V9 § 2.12.4.2 (5 bytes prepended; PTS surfaced in the PES header).
/// For `KlvStreamType::PrivateData` streams, the caller's bytes pass
/// through unchanged.
///
/// On a single-stream sender, prefer `srtc_managed_mux_sender_send_klv` —
/// same effect, no handle required.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_managed_mux_sender_send_klv_to(
    p: *mut SrtcManagedMuxSender,
    stream_handle: SrtcKlvStreamHandle,
    klv: *const u8,
    len: usize,
    pts_90khz: i64,
) -> libc::c_int {
    let Some(wrapper) = (unsafe { p.as_ref() }) else {
        set_last_error(SrtcError::InvalidConfig, "null sender pointer");
        return SrtcError::InvalidConfig as i32;
    };
    if klv.is_null() && len > 0 {
        set_last_error(SrtcError::InvalidConfig, "null klv with non-zero len");
        return SrtcError::InvalidConfig as i32;
    }
    let slice = unsafe { std::slice::from_raw_parts(klv, len) };
    let stream = KlvStreamHandle::from_raw(stream_handle);
    wrapper
        .inner
        .with_inner_ref(|s| match s.send_klv_to(stream, slice, pts_90khz) {
            Ok(()) => 0,
            Err(e) => {
                record_sender_error(&e);
                unsafe { srtc_get_last_error() }
            }
        })
}

/// Snapshot stats for a `srtc_managed_mux_sender_t` into `*out`.
///
/// Returns 0 on success, `SRTC_E_INVALID_CONFIG` if either pointer is
/// null, or `SRTC_E_CLOSED` if the sender has been closed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_managed_mux_sender_get_stats(
    p: *mut SrtcManagedMuxSender,
    out: *mut crate::stats::SrtcSenderStats,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(SrtcError::InvalidConfig, "null sender pointer");
        return SrtcError::InvalidConfig as i32;
    };
    if out.is_null() {
        set_last_error(SrtcError::InvalidConfig, "null out pointer");
        return SrtcError::InvalidConfig as i32;
    }
    handle.inner.with_inner_ref(|s| {
        let stats = s.stats();
        let mut per_stream =
            [crate::stats::SrtcStreamStats::default(); crate::stats::SRTC_STATS_MAX_STREAMS];
        let (per_stream_count, truncated) =
            crate::stats::fill_per_stream(&mut per_stream, &stats.per_stream);
        let dst = crate::stats::SrtcSenderStats {
            bytes_sent: stats.bytes_sent,
            packets_sent: stats.packets_sent,
            pending_bytes_queued: stats.pending_bytes_queued,
            pending_chunks_queued: stats.pending_chunks_queued,
            programs_configured: stats.programs_configured,
            per_stream_count,
            per_stream_truncated: if truncated { 1 } else { 0 },
            per_stream,
        };
        unsafe { *out = dst };
        0
    })
}

/// Reset stats counters for a `srtc_managed_mux_sender_t` to zero.
///
/// Returns 0 on success, `SRTC_E_INVALID_CONFIG` if the pointer is
/// null, or `SRTC_E_CLOSED` if the sender has been closed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_managed_mux_sender_reset_stats(
    p: *mut SrtcManagedMuxSender,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(SrtcError::InvalidConfig, "null sender pointer");
        return SrtcError::InvalidConfig as i32;
    };
    handle.inner.with_inner_ref(|s| {
        s.reset_stats();
        0
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_managed_mux_sender_close(p: *mut SrtcManagedMuxSender) {
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
    use std::ffi::CString;

    #[test]
    fn open_with_invalid_url_returns_null_and_sets_error() {
        unsafe {
            let cfg = srtc_mux_config_new();
            let prog = srtc_mux_config_add_program(cfg, 1, 0x1000);
            srtc_mux_config_add_video_stream(cfg, prog, 0x1011, SrtcVideoCodec::H264);
            srtc_mux_config_add_klv_stream(
                cfg,
                prog,
                0x1031,
                SrtcKlvStreamType::PrivateData,
                false,
            );
            let bad = CString::new("not-an-srt-url").unwrap();
            let p = srtc_mux_sender_open(bad.as_ptr(), cfg);
            assert!(p.is_null());
            assert_eq!(
                crate::error::srtc_get_last_error() as i32,
                SrtcError::InvalidConfig as i32,
            );
            srtc_mux_config_free(cfg);
        }
    }

    #[test]
    fn open_with_unreachable_host_returns_null_with_transport_error() {
        unsafe {
            let cfg = srtc_mux_config_new();
            let prog = srtc_mux_config_add_program(cfg, 1, 0x1000);
            srtc_mux_config_add_video_stream(cfg, prog, 0x1011, SrtcVideoCodec::H264);
            srtc_mux_config_add_klv_stream(
                cfg,
                prog,
                0x1031,
                SrtcKlvStreamType::PrivateData,
                false,
            );
            // Reserved-for-documentation address that should reject quickly.
            let url = CString::new("srt://192.0.2.1:9").unwrap();
            let p = srtc_mux_sender_open(url.as_ptr(), cfg);
            assert!(p.is_null());
            // Either Transport (broken) or InvalidConfig depending on libsrt
            // resolver behavior — both are valid failures here.
            let code = crate::error::srtc_get_last_error() as i32;
            assert!(
                code == SrtcError::Transport as i32 || code == SrtcError::InvalidConfig as i32,
                "expected Transport or InvalidConfig, got {code}",
            );
            srtc_mux_config_free(cfg);
        }
    }

    #[test]
    fn null_close_is_safe() {
        unsafe {
            srtc_mux_sender_close(std::ptr::null_mut());
            srtc_managed_mux_sender_close(std::ptr::null_mut());
        }
    }
}
