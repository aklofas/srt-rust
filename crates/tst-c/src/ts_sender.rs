//! `tst_sender_t` and `tst_managed_sender_t`.
//!
//! Pre-muxed TS bytes -> SRT, with sync-byte framing/recovery (RECOVER or
//! STRICT mode per `tst_sender_config_t::framing_mode`).

use crate::config::{TstReconnectPolicy, TstSenderConfig};
use crate::error::{TstError, record_shell_error, record_transport_error, set_last_error};
use crate::handle::Handle;
use crate::mux_sender::parse_c_srt_url;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tst_pipeline::{ManagedTransport, Sender, SenderStats, TransportCancel};
use tst_srt::SrtTransport;

/// Public-ABI mirror of `tst_pipeline::SenderStats`. Same fields,
/// same units. Caller passes a pointer to a stack-allocated struct;
/// `tst_sender_get_stats` fills it in.
#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct TstSenderStats {
    pub bytes_pushed: u64,
    pub bytes_skipped_for_sync: u64,
    pub resync_events: u64,
    pub packets_sent: u64,
}

impl From<&SenderStats> for TstSenderStats {
    fn from(s: &SenderStats) -> Self {
        Self {
            bytes_pushed: s.bytes_pushed,
            bytes_skipped_for_sync: s.bytes_skipped_for_sync,
            resync_events: s.resync_events,
            packets_sent: s.packets_sent,
        }
    }
}

// ------------------------------------------------------------------
// tst_sender_t (plain L1)
// ------------------------------------------------------------------

pub struct TstSender {
    inner: Handle<Sender<SrtTransport>>,
    cancel: Option<Arc<dyn TransportCancel + Send + Sync>>,
    /// Informational only on the sender side — set by `_cancel` and `_close`
    /// but never read by `_send` paths. Kept for shape uniformity with the
    /// receiver structs (where it gates peer-FIN vs caller-close discrimination
    /// in `_recv`); future JNI/UniFFI bindings reflecting on field types see
    /// the same shape across all 8 handle families.
    was_cancelled: Arc<AtomicBool>,
}

/// Open a `tst_sender_t` connected via SRT.
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
pub unsafe extern "C" fn tst_sender_open(
    srt_url: *const libc::c_char,
    cfg: *const TstSenderConfig,
) -> *mut TstSender {
    crate::panic::ffi_catch(std::ptr::null_mut(), || {
        let cfg = match unsafe { cfg.as_ref() } {
            Some(c) => c.inner.clone(),
            None => tst_pipeline::SenderConfig::default(),
        };
        let url = match unsafe { parse_c_srt_url(srt_url) } {
            Ok(u) => u,
            Err(()) => return std::ptr::null_mut(),
        };
        let mut socket_cfg = tst_srt::config::SocketConfig::default();
        url.overlay.apply_to_socket(&mut socket_cfg);
        let transport = match crate::connect::connect_srt(&url.host, url.port, &socket_cfg) {
            Ok(t) => t,
            Err(e) => {
                record_transport_error(&e);
                return std::ptr::null_mut();
            }
        };
        let sender = Sender::new(transport, cfg);
        let cancel = sender.cancel_handle();
        let was_cancelled = Arc::new(AtomicBool::new(false));
        Box::into_raw(Box::new(TstSender {
            inner: Handle::new(sender),
            cancel,
            was_cancelled,
        }))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_sender_send_ts(
    p: *mut TstSender,
    bytes: *const u8,
    len: usize,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null sender pointer");
        return TstError::InvalidConfig as i32;
    };
    if bytes.is_null() && len > 0 {
        set_last_error(TstError::InvalidConfig, "null bytes with non-zero len");
        return TstError::InvalidConfig as i32;
    }
    let slice = unsafe { std::slice::from_raw_parts(bytes, len) };
    handle.inner.with_inner_mut(|s| match s.send_ts(slice) {
        Ok(()) => 0,
        Err(e) => record_shell_error(&e),
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_sender_flush(p: *mut TstSender) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null sender pointer");
        return TstError::InvalidConfig as i32;
    };
    handle.inner.with_inner_mut(|s| match s.flush() {
        Ok(()) => 0,
        Err(e) => record_shell_error(&e),
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_sender_get_stats(
    p: *mut TstSender,
    out: *mut TstSenderStats,
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
        let stats = TstSenderStats::from(&s.stats());
        unsafe { *out = stats };
        0
    })
}

/// Read wire-level transport stats for the underlying libsrt socket.
/// See [`tst_mux_sender_get_socket_stats`](crate::mux_sender::tst_mux_sender_get_socket_stats)
/// for full semantics — same shape, different handle type.
///
/// # Safety
///
/// Caller MUST ensure `p` is a valid `*mut TstSender` opened via
/// `tst_sender_open` and `out` points to a writable `TstSocketStats`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_sender_get_socket_stats(
    p: *mut TstSender,
    out: *mut crate::stats::TstSocketStats,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null sender pointer");
        return TstError::InvalidConfig as i32;
    };
    if out.is_null() {
        set_last_error(TstError::InvalidConfig, "null out pointer");
        return TstError::InvalidConfig as i32;
    }
    unsafe { *out = crate::stats::TstSocketStats::default() };
    handle.inner.with_inner_ref(|s| match s.socket_stats() {
        Some(stats) => {
            unsafe { *out = (&stats).into() };
            0
        }
        None => TstError::NotAvailable as i32,
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_sender_reset_stats(p: *mut TstSender) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null sender pointer");
        return TstError::InvalidConfig as i32;
    };
    handle.inner.with_inner_mut(|s| {
        s.reset_stats();
        0
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_sender_close(p: *mut TstSender) {
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

/// Cancel a `tst_sender_t`. Unblocks a thread parked in `_send`
/// within one libsrt I/O cycle (~3-10 ms) by closing the underlying
/// libsrt socket. Safe to call from any thread. Idempotent.
///
/// Returns 0 on success, `TST_E_INVALID_CONFIG` if the pointer is null.
///
/// After cancel, `_send` returns `TST_E_CLOSED`. The handle must still
/// be `_close`'d to free.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_sender_cancel(p: *mut TstSender) -> libc::c_int {
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

// ------------------------------------------------------------------
// tst_managed_sender_t (managed L2)
// ------------------------------------------------------------------

pub struct TstManagedSender {
    inner: Handle<Sender<ManagedTransport<SrtTransport>>>,
    cancel: Option<Arc<dyn TransportCancel + Send + Sync>>,
    /// Informational only on the sender side — set by `_cancel` and `_close`
    /// but never read by `_send` paths. Kept for shape uniformity with the
    /// receiver structs (where it gates peer-FIN vs caller-close discrimination
    /// in `_recv`); future JNI/UniFFI bindings reflecting on field types see
    /// the same shape across all 8 handle families.
    was_cancelled: Arc<AtomicBool>,
}

/// Open a `tst_managed_sender_t` connected via SRT.
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
pub unsafe extern "C" fn tst_managed_sender_open(
    srt_url: *const libc::c_char,
    cfg: *const TstSenderConfig,
    policy: *const TstReconnectPolicy,
) -> *mut TstManagedSender {
    crate::panic::ffi_catch(std::ptr::null_mut(), || {
        let cfg = match unsafe { cfg.as_ref() } {
            Some(c) => c.inner.clone(),
            None => tst_pipeline::SenderConfig::default(),
        };
        let policy = match unsafe { policy.as_ref() } {
            Some(p) => p.inner.clone(),
            None => tst_pipeline::ReconnectPolicy::default(),
        };
        let url = match unsafe { parse_c_srt_url(srt_url) } {
            Ok(u) => u,
            Err(()) => return std::ptr::null_mut(),
        };
        let mut socket_cfg = tst_srt::config::SocketConfig::default();
        url.overlay.apply_to_socket(&mut socket_cfg);

        let initial = match crate::connect::connect_srt(&url.host, url.port, &socket_cfg) {
            Ok(t) => t,
            Err(e) => {
                record_transport_error(&e);
                return std::ptr::null_mut();
            }
        };
        let host = url.host.clone();
        let port = url.port;
        let cfg_for_reconnect = socket_cfg.clone();
        let factory = move || crate::connect::connect_srt(&host, port, &cfg_for_reconnect);
        let managed = ManagedTransport::new(initial, factory, policy);
        let sender = Sender::new(managed, cfg);
        let cancel = sender.cancel_handle();
        let was_cancelled = Arc::new(AtomicBool::new(false));
        Box::into_raw(Box::new(TstManagedSender {
            inner: Handle::new(sender),
            cancel,
            was_cancelled,
        }))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_managed_sender_send_ts(
    p: *mut TstManagedSender,
    bytes: *const u8,
    len: usize,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null sender pointer");
        return TstError::InvalidConfig as i32;
    };
    if bytes.is_null() && len > 0 {
        set_last_error(TstError::InvalidConfig, "null bytes with non-zero len");
        return TstError::InvalidConfig as i32;
    }
    let slice = unsafe { std::slice::from_raw_parts(bytes, len) };
    handle.inner.with_inner_mut(|s| match s.send_ts(slice) {
        Ok(()) => 0,
        Err(e) => record_shell_error(&e),
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_managed_sender_flush(p: *mut TstManagedSender) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null sender pointer");
        return TstError::InvalidConfig as i32;
    };
    handle.inner.with_inner_mut(|s| match s.flush() {
        Ok(()) => 0,
        Err(e) => record_shell_error(&e),
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_managed_sender_get_stats(
    p: *mut TstManagedSender,
    out: *mut TstSenderStats,
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
        let stats = TstSenderStats::from(&s.stats());
        unsafe { *out = stats };
        0
    })
}

/// Managed sibling of [`tst_sender_get_socket_stats`]. Returns
/// `TST_E_NOT_AVAILABLE` when the reconnect loop currently has no live
/// inner socket.
///
/// # Safety
///
/// Caller MUST ensure `p` is a valid `*mut TstManagedSender` opened via
/// `tst_managed_sender_open` and `out` points to a writable
/// `TstSocketStats`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_managed_sender_get_socket_stats(
    p: *mut TstManagedSender,
    out: *mut crate::stats::TstSocketStats,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null sender pointer");
        return TstError::InvalidConfig as i32;
    };
    if out.is_null() {
        set_last_error(TstError::InvalidConfig, "null out pointer");
        return TstError::InvalidConfig as i32;
    }
    unsafe { *out = crate::stats::TstSocketStats::default() };
    handle.inner.with_inner_ref(|s| match s.socket_stats() {
        Some(stats) => {
            unsafe { *out = (&stats).into() };
            0
        }
        None => TstError::NotAvailable as i32,
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_managed_sender_reset_stats(p: *mut TstManagedSender) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null sender pointer");
        return TstError::InvalidConfig as i32;
    };
    handle.inner.with_inner_mut(|s| {
        s.reset_stats();
        0
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_managed_sender_close(p: *mut TstManagedSender) {
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

/// Cancel a `tst_managed_sender_t`. Same semantics as
/// `tst_sender_cancel`; reaches the currently-active inner
/// transport's cancel handle through `ManagedTransport`'s atomic
/// snapshot.
///
/// Returns 0 on success, `TST_E_INVALID_CONFIG` if the pointer is null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_managed_sender_cancel(p: *mut TstManagedSender) -> libc::c_int {
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

    #[test]
    fn open_with_null_url_returns_null() {
        unsafe {
            let cfg = tst_sender_config_new();
            let p = tst_sender_open(std::ptr::null(), cfg);
            assert!(p.is_null());
            tst_sender_config_free(cfg);
        }
    }

    #[test]
    fn null_close_is_safe() {
        unsafe {
            tst_sender_close(std::ptr::null_mut());
            tst_managed_sender_close(std::ptr::null_mut());
        }
    }

    #[test]
    fn null_cancel_returns_invalid_config() {
        let rc = unsafe { tst_sender_cancel(std::ptr::null_mut()) };
        assert_eq!(rc, TstError::InvalidConfig as i32);
    }

    #[test]
    fn managed_null_cancel_returns_invalid_config() {
        let rc = unsafe { tst_managed_sender_cancel(std::ptr::null_mut()) };
        assert_eq!(rc, TstError::InvalidConfig as i32);
    }
}
