//! `tst_mux_sender_t` (plain) and `tst_managed_mux_sender_t` (managed).
//!
//! Both wrap `tst_pipeline::MuxSender<T>`, with T parameterized on the
//! underlying transport. Plain uses `SrtTransport`; managed uses
//! `ManagedTransport<SrtTransport>` with a factory that reconnects via the
//! original URL on transport breakage.

use crate::config::{TstMuxConfig, TstReconnectPolicy};
use crate::error::{
    TstError, record_mux_error, record_sender_error, set_last_error, tst_get_last_error,
};
use crate::handle::{Handle, TstKlvStreamHandle, TstVideoStreamHandle};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tst_core::mpegts::mux::{KlvStreamHandle, VideoStreamHandle};
use tst_pipeline::{ManagedTransport, MuxSender, TransportCancel};
use tst_srt::SrtTransport;
use tst_srt::config::SocketConfig;

// ------------------------------------------------------------------
// tst_mux_sender_t (plain L1)
// ------------------------------------------------------------------

pub struct TstMuxSender {
    inner: Handle<MuxSender<SrtTransport>>,
    cancel: Option<Arc<dyn TransportCancel + Send + Sync>>,
    /// Informational only on the sender side — set by `_cancel` and `_close`
    /// but never read by `_send` paths. Kept for shape uniformity with the
    /// receiver structs (where it gates peer-FIN vs caller-close discrimination
    /// in `_recv`); future JNI/UniFFI bindings reflecting on field types see
    /// the same shape across all 8 handle families.
    was_cancelled: Arc<AtomicBool>,
}

/// Open a `tst_mux_sender_t` connected via SRT.
///
/// `srt_url` is a `srt://host:port?key=value&...` URL. Query
/// parameters apply libsrt-vocabulary options to the connection
/// (passphrase, latency, streamid, etc.). URL values override config
/// values for the same option. See
/// `docs/guide-srt.md#url-parsing` for the recognized key table.
///
/// Returns `NULL` with `TST_E_INVALID_CONFIG` set in the thread-local
/// last-error for any malformed URL, unsupported key, unknown key, or
/// invalid value. The detail string from
/// `tst_get_last_error_str()` describes the specific problem.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_mux_sender_open(
    srt_url: *const libc::c_char,
    cfg: *mut TstMuxConfig,
) -> *mut TstMuxSender {
    crate::panic::ffi_catch(std::ptr::null_mut(), || {
        let Some(cfg) = (unsafe { cfg.as_mut() }) else {
            set_last_error(TstError::InvalidConfig, "null config pointer");
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
        let sender = match MuxSender::new(transport, built) {
            Ok(s) => s,
            Err(e) => {
                record_mux_error(&e);
                return std::ptr::null_mut();
            }
        };
        let cancel = sender.cancel_handle();
        let was_cancelled = Arc::new(AtomicBool::new(false));
        Box::into_raw(Box::new(TstMuxSender {
            inner: Handle::new(sender),
            cancel,
            was_cancelled,
        }))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_mux_sender_send_video(
    p: *mut TstMuxSender,
    nal: *const u8,
    len: usize,
    pts_90khz: i64,
    key_frame: bool,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null sender pointer");
        return TstError::InvalidConfig as i32;
    };
    if nal.is_null() && len > 0 {
        set_last_error(TstError::InvalidConfig, "null nal with non-zero len");
        return TstError::InvalidConfig as i32;
    }
    let slice = unsafe { std::slice::from_raw_parts(nal, len) };
    handle
        .inner
        .with_inner_ref(|s| match s.send_video(slice, pts_90khz, key_frame) {
            Ok(()) => 0,
            Err(e) => {
                record_sender_error(&e);
                unsafe { tst_get_last_error() }
            }
        })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_mux_sender_send_klv(
    p: *mut TstMuxSender,
    klv: *const u8,
    len: usize,
    pts_90khz: i64,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null sender pointer");
        return TstError::InvalidConfig as i32;
    };
    if klv.is_null() && len > 0 {
        set_last_error(TstError::InvalidConfig, "null klv with non-zero len");
        return TstError::InvalidConfig as i32;
    }
    let slice = unsafe { std::slice::from_raw_parts(klv, len) };
    handle.inner.with_inner_ref(|s| {
        match s.send_klv(
            slice, pts_90khz,
            // C ABI receiver-surface plan will expose metadata_service_id;
            // today defaults to 0x00 per ST 1402.2 App. B Table 2.
            0x00,
        ) {
            Ok(()) => 0,
            Err(e) => {
                record_sender_error(&e);
                unsafe { tst_get_last_error() }
            }
        }
    })
}

/// Push one Annex-B NAL targeting a specific video elementary stream.
///
/// `stream_handle` is obtained from `tst_mux_config_add_video_stream` at
/// config time and is stable across the config→open boundary. Out-of-range
/// handles surface as `TST_E_INVALID_USAGE` (carrying
/// `MuxError::InvalidStreamHandle`).
///
/// On a single-stream sender, prefer `tst_mux_sender_send_video` — same
/// effect, no handle required.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_mux_sender_send_video_to(
    p: *mut TstMuxSender,
    stream_handle: TstVideoStreamHandle,
    nal: *const u8,
    len: usize,
    pts_90khz: i64,
    key_frame: bool,
) -> libc::c_int {
    let Some(wrapper) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null sender pointer");
        return TstError::InvalidConfig as i32;
    };
    if nal.is_null() && len > 0 {
        set_last_error(TstError::InvalidConfig, "null nal with non-zero len");
        return TstError::InvalidConfig as i32;
    }
    let slice = unsafe { std::slice::from_raw_parts(nal, len) };
    let stream = VideoStreamHandle::from_raw(stream_handle);
    wrapper.inner.with_inner_ref(
        |s| match s.send_video_to(stream, slice, pts_90khz, key_frame) {
            Ok(()) => 0,
            Err(e) => {
                record_sender_error(&e);
                unsafe { tst_get_last_error() }
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
/// On a single-stream sender, prefer `tst_mux_sender_send_klv` — same
/// effect, no handle required.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_mux_sender_send_klv_to(
    p: *mut TstMuxSender,
    stream_handle: TstKlvStreamHandle,
    klv: *const u8,
    len: usize,
    pts_90khz: i64,
) -> libc::c_int {
    let Some(wrapper) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null sender pointer");
        return TstError::InvalidConfig as i32;
    };
    if klv.is_null() && len > 0 {
        set_last_error(TstError::InvalidConfig, "null klv with non-zero len");
        return TstError::InvalidConfig as i32;
    }
    let slice = unsafe { std::slice::from_raw_parts(klv, len) };
    let stream = KlvStreamHandle::from_raw(stream_handle);
    wrapper.inner.with_inner_ref(|s| {
        match s.send_klv_to(
            stream, slice, pts_90khz,
            // C ABI receiver-surface plan will expose metadata_service_id;
            // today defaults to 0x00 per ST 1402.2 App. B Table 2.
            0x00,
        ) {
            Ok(()) => 0,
            Err(e) => {
                record_sender_error(&e);
                unsafe { tst_get_last_error() }
            }
        }
    })
}

/// Snapshot stats for a `tst_mux_sender_t` into `*out`.
///
/// Returns 0 on success, `TST_E_INVALID_CONFIG` if either pointer is
/// null, or `TST_E_CLOSED` if the sender has been closed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_mux_sender_get_stats(
    p: *mut TstMuxSender,
    out: *mut crate::stats::TstMuxSenderStats,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null sender pointer");
        return TstError::InvalidConfig as i32;
    };
    if out.is_null() {
        set_last_error(TstError::InvalidConfig, "null out pointer");
        return TstError::InvalidConfig as i32;
    }
    handle.inner.with_inner_ref(|s| {
        let stats = s.stats();
        let mut per_stream =
            [crate::stats::TstStreamStats::default(); crate::stats::TST_STATS_MAX_STREAMS];
        let (per_stream_count, truncated) =
            crate::stats::fill_per_stream(&mut per_stream, &stats.per_stream);
        let dst = crate::stats::TstMuxSenderStats {
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

/// Reset stats counters for a `tst_mux_sender_t` to zero.
///
/// Returns 0 on success, `TST_E_INVALID_CONFIG` if the pointer is
/// null, or `TST_E_CLOSED` if the sender has been closed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_mux_sender_reset_stats(p: *mut TstMuxSender) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null sender pointer");
        return TstError::InvalidConfig as i32;
    };
    handle.inner.with_inner_ref(|s| {
        s.reset_stats();
        0
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_mux_sender_close(p: *mut TstMuxSender) {
    if p.is_null() {
        return;
    }
    let boxed = unsafe { Box::from_raw(p) };
    boxed.was_cancelled.store(true, Ordering::Release);
    if let Some(c) = &boxed.cancel {
        c.cancel();
    }
    boxed.inner.close();
    drop(boxed);
}

/// Cancel a `tst_mux_sender_t`. Unblocks a thread parked in any `_send_*`
/// entry point within one libsrt I/O cycle (~3-10 ms) by closing the
/// underlying libsrt socket. Safe to call from any thread. Idempotent.
///
/// Returns 0 on success, `TST_E_INVALID_CONFIG` if the pointer is null.
///
/// After cancel, all `_send_*` entry points return `TST_E_CLOSED`. The
/// handle must still be `_close`'d to free.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_mux_sender_cancel(p: *mut TstMuxSender) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null sender pointer");
        return TstError::InvalidConfig as i32;
    };
    // Side-channel: do NOT acquire handle.inner's Mutex (a concurrent
    // send holds it). The was_cancelled flag + cancel-handle Arc are
    // accessible without locking.
    handle.was_cancelled.store(true, Ordering::Release);
    if let Some(c) = &handle.cancel {
        c.cancel();
    }
    0
}

/// Borrow `srt_url` as a Rust string and run it through `tst_srt::url`'s
/// rich URL parser. Sets last-error and returns `Err(())` on any failure
/// path; caller treats `Err(())` as "return NULL".
pub(crate) unsafe fn parse_c_srt_url(srt_url: *const libc::c_char) -> Result<tst_srt::SrtUrl, ()> {
    if srt_url.is_null() {
        set_last_error(TstError::InvalidConfig, "null srt_url");
        return Err(());
    }
    let cstr = unsafe { std::ffi::CStr::from_ptr(srt_url) };
    let s = match cstr.to_str() {
        Ok(s) => s,
        Err(_) => {
            set_last_error(TstError::InvalidConfig, "srt_url is not valid utf-8");
            return Err(());
        }
    };
    tst_srt::SrtUrl::parse(s).map_err(|e| {
        set_last_error(TstError::InvalidConfig, &format!("invalid srt url: {e}"));
    })
}

// ------------------------------------------------------------------
// tst_managed_mux_sender_t (managed L2)
// ------------------------------------------------------------------

pub struct TstManagedMuxSender {
    inner: Handle<MuxSender<ManagedTransport<SrtTransport>>>,
    cancel: Option<Arc<dyn TransportCancel + Send + Sync>>,
    /// Informational only on the sender side — set by `_cancel` and `_close`
    /// but never read by `_send` paths. Kept for shape uniformity with the
    /// receiver structs (where it gates peer-FIN vs caller-close discrimination
    /// in `_recv`); future JNI/UniFFI bindings reflecting on field types see
    /// the same shape across all 8 handle families.
    was_cancelled: Arc<AtomicBool>,
}

/// Open a `tst_managed_mux_sender_t` connected via SRT.
///
/// `srt_url` is a `srt://host:port?key=value&...` URL. Query
/// parameters apply libsrt-vocabulary options to the connection
/// (passphrase, latency, streamid, etc.). URL values override config
/// values for the same option. See
/// `docs/guide-srt.md#url-parsing` for the recognized key table.
///
/// Returns `NULL` with `TST_E_INVALID_CONFIG` set in the thread-local
/// last-error for any malformed URL, unsupported key, unknown key, or
/// invalid value. The detail string from
/// `tst_get_last_error_str()` describes the specific problem.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_managed_mux_sender_open(
    srt_url: *const libc::c_char,
    cfg: *mut TstMuxConfig,
    policy: *const TstReconnectPolicy,
) -> *mut TstManagedMuxSender {
    crate::panic::ffi_catch(std::ptr::null_mut(), || {
        let Some(cfg) = (unsafe { cfg.as_mut() }) else {
            set_last_error(TstError::InvalidConfig, "null config pointer");
            return std::ptr::null_mut();
        };
        let policy = match unsafe { policy.as_ref() } {
            Some(p) => p.inner.clone(),
            None => tst_pipeline::ReconnectPolicy::default(),
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
        let sender = match MuxSender::new(managed, built) {
            Ok(s) => s,
            Err(e) => {
                record_mux_error(&e);
                return std::ptr::null_mut();
            }
        };
        let cancel = sender.cancel_handle();
        let was_cancelled = Arc::new(AtomicBool::new(false));
        Box::into_raw(Box::new(TstManagedMuxSender {
            inner: Handle::new(sender),
            cancel,
            was_cancelled,
        }))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_managed_mux_sender_send_video(
    p: *mut TstManagedMuxSender,
    nal: *const u8,
    len: usize,
    pts_90khz: i64,
    key_frame: bool,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null sender pointer");
        return TstError::InvalidConfig as i32;
    };
    if nal.is_null() && len > 0 {
        set_last_error(TstError::InvalidConfig, "null nal with non-zero len");
        return TstError::InvalidConfig as i32;
    }
    let slice = unsafe { std::slice::from_raw_parts(nal, len) };
    handle
        .inner
        .with_inner_ref(|s| match s.send_video(slice, pts_90khz, key_frame) {
            Ok(()) => 0,
            Err(e) => {
                record_sender_error(&e);
                unsafe { tst_get_last_error() }
            }
        })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_managed_mux_sender_send_klv(
    p: *mut TstManagedMuxSender,
    klv: *const u8,
    len: usize,
    pts_90khz: i64,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null sender pointer");
        return TstError::InvalidConfig as i32;
    };
    if klv.is_null() && len > 0 {
        set_last_error(TstError::InvalidConfig, "null klv with non-zero len");
        return TstError::InvalidConfig as i32;
    }
    let slice = unsafe { std::slice::from_raw_parts(klv, len) };
    handle.inner.with_inner_ref(|s| {
        match s.send_klv(
            slice, pts_90khz,
            // C ABI receiver-surface plan will expose metadata_service_id;
            // today defaults to 0x00 per ST 1402.2 App. B Table 2.
            0x00,
        ) {
            Ok(()) => 0,
            Err(e) => {
                record_sender_error(&e);
                unsafe { tst_get_last_error() }
            }
        }
    })
}

/// Push one Annex-B NAL targeting a specific video elementary stream on a
/// managed (auto-reconnecting) sender.
///
/// `stream_handle` is obtained from `tst_mux_config_add_video_stream` at
/// config time and is stable across reconnects. Out-of-range handles
/// surface as `TST_E_INVALID_USAGE` (carrying
/// `MuxError::InvalidStreamHandle`).
///
/// On a single-stream sender, prefer `tst_managed_mux_sender_send_video` —
/// same effect, no handle required.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_managed_mux_sender_send_video_to(
    p: *mut TstManagedMuxSender,
    stream_handle: TstVideoStreamHandle,
    nal: *const u8,
    len: usize,
    pts_90khz: i64,
    key_frame: bool,
) -> libc::c_int {
    let Some(wrapper) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null sender pointer");
        return TstError::InvalidConfig as i32;
    };
    if nal.is_null() && len > 0 {
        set_last_error(TstError::InvalidConfig, "null nal with non-zero len");
        return TstError::InvalidConfig as i32;
    }
    let slice = unsafe { std::slice::from_raw_parts(nal, len) };
    let stream = VideoStreamHandle::from_raw(stream_handle);
    wrapper.inner.with_inner_ref(
        |s| match s.send_video_to(stream, slice, pts_90khz, key_frame) {
            Ok(()) => 0,
            Err(e) => {
                record_sender_error(&e);
                unsafe { tst_get_last_error() }
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
/// On a single-stream sender, prefer `tst_managed_mux_sender_send_klv` —
/// same effect, no handle required.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_managed_mux_sender_send_klv_to(
    p: *mut TstManagedMuxSender,
    stream_handle: TstKlvStreamHandle,
    klv: *const u8,
    len: usize,
    pts_90khz: i64,
) -> libc::c_int {
    let Some(wrapper) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null sender pointer");
        return TstError::InvalidConfig as i32;
    };
    if klv.is_null() && len > 0 {
        set_last_error(TstError::InvalidConfig, "null klv with non-zero len");
        return TstError::InvalidConfig as i32;
    }
    let slice = unsafe { std::slice::from_raw_parts(klv, len) };
    let stream = KlvStreamHandle::from_raw(stream_handle);
    wrapper.inner.with_inner_ref(|s| {
        match s.send_klv_to(
            stream, slice, pts_90khz,
            // C ABI receiver-surface plan will expose metadata_service_id;
            // today defaults to 0x00 per ST 1402.2 App. B Table 2.
            0x00,
        ) {
            Ok(()) => 0,
            Err(e) => {
                record_sender_error(&e);
                unsafe { tst_get_last_error() }
            }
        }
    })
}

/// Snapshot stats for a `tst_managed_mux_sender_t` into `*out`.
///
/// Returns 0 on success, `TST_E_INVALID_CONFIG` if either pointer is
/// null, or `TST_E_CLOSED` if the sender has been closed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_managed_mux_sender_get_stats(
    p: *mut TstManagedMuxSender,
    out: *mut crate::stats::TstMuxSenderStats,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null sender pointer");
        return TstError::InvalidConfig as i32;
    };
    if out.is_null() {
        set_last_error(TstError::InvalidConfig, "null out pointer");
        return TstError::InvalidConfig as i32;
    }
    handle.inner.with_inner_ref(|s| {
        let stats = s.stats();
        let mut per_stream =
            [crate::stats::TstStreamStats::default(); crate::stats::TST_STATS_MAX_STREAMS];
        let (per_stream_count, truncated) =
            crate::stats::fill_per_stream(&mut per_stream, &stats.per_stream);
        let dst = crate::stats::TstMuxSenderStats {
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

/// Reset stats counters for a `tst_managed_mux_sender_t` to zero.
///
/// Returns 0 on success, `TST_E_INVALID_CONFIG` if the pointer is
/// null, or `TST_E_CLOSED` if the sender has been closed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_managed_mux_sender_reset_stats(
    p: *mut TstManagedMuxSender,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null sender pointer");
        return TstError::InvalidConfig as i32;
    };
    handle.inner.with_inner_ref(|s| {
        s.reset_stats();
        0
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_managed_mux_sender_close(p: *mut TstManagedMuxSender) {
    if p.is_null() {
        return;
    }
    let boxed = unsafe { Box::from_raw(p) };
    boxed.was_cancelled.store(true, Ordering::Release);
    if let Some(c) = &boxed.cancel {
        c.cancel();
    }
    boxed.inner.close();
    drop(boxed);
}

/// Cancel a `tst_managed_mux_sender_t`. Same semantics as
/// `tst_mux_sender_cancel`; reaches the currently-active inner
/// transport's cancel handle through `ManagedTransport`'s atomic
/// snapshot.
///
/// Returns 0 on success, `TST_E_INVALID_CONFIG` if the pointer is null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_managed_mux_sender_cancel(
    p: *mut TstManagedMuxSender,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null sender pointer");
        return TstError::InvalidConfig as i32;
    };
    // Side-channel: do NOT acquire handle.inner's Mutex (a concurrent
    // send holds it). The was_cancelled flag + cancel-handle Arc are
    // accessible without locking.
    handle.was_cancelled.store(true, Ordering::Release);
    if let Some(c) = &handle.cancel {
        c.cancel();
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::*;
    use std::ffi::CString;

    #[test]
    fn open_with_invalid_url_returns_null_and_sets_error() {
        unsafe {
            let cfg = tst_mux_config_new();
            let prog = tst_mux_config_add_program(cfg, 1, 0x1000);
            tst_mux_config_add_video_stream(cfg, prog, 0x1011, TstVideoCodec::H264);
            tst_mux_config_add_klv_stream(cfg, prog, 0x1031, TstKlvStreamType::PrivateData, false);
            let bad = CString::new("not-an-srt-url").unwrap();
            let p = tst_mux_sender_open(bad.as_ptr(), cfg);
            assert!(p.is_null());
            assert_eq!(
                crate::error::tst_get_last_error() as i32,
                TstError::InvalidConfig as i32,
            );
            tst_mux_config_free(cfg);
        }
    }

    #[test]
    fn open_with_unreachable_host_returns_null_with_transport_error() {
        unsafe {
            let cfg = tst_mux_config_new();
            let prog = tst_mux_config_add_program(cfg, 1, 0x1000);
            tst_mux_config_add_video_stream(cfg, prog, 0x1011, TstVideoCodec::H264);
            tst_mux_config_add_klv_stream(cfg, prog, 0x1031, TstKlvStreamType::PrivateData, false);
            // Reserved-for-documentation address that should reject quickly.
            let url = CString::new("srt://192.0.2.1:9").unwrap();
            let p = tst_mux_sender_open(url.as_ptr(), cfg);
            assert!(p.is_null());
            // Either Transport (broken) or InvalidConfig depending on libsrt
            // resolver behavior — both are valid failures here.
            let code = crate::error::tst_get_last_error() as i32;
            assert!(
                code == TstError::Transport as i32 || code == TstError::InvalidConfig as i32,
                "expected Transport or InvalidConfig, got {code}",
            );
            tst_mux_config_free(cfg);
        }
    }

    #[test]
    fn null_close_is_safe() {
        unsafe {
            tst_mux_sender_close(std::ptr::null_mut());
            tst_managed_mux_sender_close(std::ptr::null_mut());
        }
    }

    #[test]
    fn null_cancel_returns_invalid_config() {
        let rc = unsafe { tst_mux_sender_cancel(std::ptr::null_mut()) };
        assert_eq!(rc, TstError::InvalidConfig as i32);
    }

    #[test]
    fn managed_null_cancel_returns_invalid_config() {
        let rc = unsafe { tst_managed_mux_sender_cancel(std::ptr::null_mut()) };
        assert_eq!(rc, TstError::InvalidConfig as i32);
    }
}
