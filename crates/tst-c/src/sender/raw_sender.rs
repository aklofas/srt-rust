//! `tst_raw_sender_t` (plain) and `tst_managed_raw_sender_t` (managed).
//!
//! One _send call = one outbound SRT message of the exact length passed in.

use crate::config::{TstRawSenderConfig, TstReconnectPolicy};
use crate::error::{
    TstError, record_not_available, record_shell_error, record_transport_error, set_last_error,
};
use crate::handle::Handle;
use crate::sender::mux_sender::parse_c_srt_url;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tst_pipeline::{ManagedTransport, RawSender, TransportCancel};
use tst_srt::SrtTransport;

// ------------------------------------------------------------------
// tst_raw_sender_t
// ------------------------------------------------------------------

pub struct TstRawSender {
    inner: Handle<RawSender<SrtTransport>>,
    cancel: Option<Arc<dyn TransportCancel + Send + Sync>>,
    /// Informational only on the sender side — set by `_cancel` and `_close`
    /// but never read by `_send` paths. Kept for shape uniformity with the
    /// receiver structs (where it gates peer-FIN vs caller-close discrimination
    /// in `_recv`); future JNI/UniFFI bindings reflecting on field types see
    /// the same shape across all 8 handle families.
    was_cancelled: Arc<AtomicBool>,
}

/// Open a `tst_raw_sender_t` connected via SRT.
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
pub unsafe extern "C" fn tst_raw_sender_open(
    srt_url: *const libc::c_char,
    cfg: *const TstRawSenderConfig,
) -> *mut TstRawSender {
    crate::panic::ffi_catch(std::ptr::null_mut(), || {
        let cfg = match unsafe { cfg.as_ref() } {
            Some(c) => c.inner.clone(),
            None => tst_pipeline::RawSenderConfig::default(),
        };
        let url = match unsafe { parse_c_srt_url(srt_url) } {
            Ok(u) => u,
            Err(()) => return std::ptr::null_mut(),
        };
        let mut socket_cfg = tst_srt::config::SocketConfig::default();
        url.overlay.apply_to_socket(&mut socket_cfg);
        let transport = match crate::sender::connect::connect_srt(&url.host, url.port, &socket_cfg)
        {
            Ok(t) => t,
            Err(e) => {
                record_transport_error(&e);
                return std::ptr::null_mut();
            }
        };
        let sender = RawSender::new(transport, cfg);
        let cancel = sender.cancel_handle();
        let was_cancelled = Arc::new(AtomicBool::new(false));
        Box::into_raw(Box::new(TstRawSender {
            inner: Handle::new(sender),
            cancel,
            was_cancelled,
        }))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_raw_sender_send(
    p: *mut TstRawSender,
    bytes: *const u8,
    len: usize,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null sender pointer");
        return TstError::InvalidConfig as i32;
    };
    let slice = match unsafe { crate::ffi_slice::ffi_slice(bytes, len, "bytes") } {
        Ok(s) => s,
        Err(code) => return code,
    };
    handle.inner.with_inner_mut(|s| match s.send(slice) {
        Ok(()) => 0,
        Err(e) => record_shell_error(&e),
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_raw_sender_close(p: *mut TstRawSender) {
    crate::panic::ffi_catch((), || {
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
    });
}

/// Cancel a `tst_raw_sender_t`. Unblocks a thread parked in `_send`
/// within one libsrt I/O cycle (~3-10 ms) by closing the underlying
/// libsrt socket. Safe to call from any thread. Idempotent.
///
/// Returns 0 on success, `TST_E_INVALID_CONFIG` if the pointer is null.
///
/// After cancel, `_send` returns `TST_E_CLOSED`. The handle must still
/// be `_close`'d to free.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_raw_sender_cancel(p: *mut TstRawSender) -> libc::c_int {
    crate::panic::ffi_catch(TstError::Internal as i32, || {
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
    })
}

// ------------------------------------------------------------------
// tst_managed_raw_sender_t
// ------------------------------------------------------------------

pub struct TstManagedRawSender {
    inner: Handle<RawSender<ManagedTransport<SrtTransport>>>,
    cancel: Option<Arc<dyn TransportCancel + Send + Sync>>,
    /// Informational only on the sender side — set by `_cancel` and `_close`
    /// but never read by `_send` paths. Kept for shape uniformity with the
    /// receiver structs (where it gates peer-FIN vs caller-close discrimination
    /// in `_recv`); future JNI/UniFFI bindings reflecting on field types see
    /// the same shape across all 8 handle families.
    was_cancelled: Arc<AtomicBool>,
}

/// Open a `tst_managed_raw_sender_t` connected via SRT.
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
pub unsafe extern "C" fn tst_managed_raw_sender_open(
    srt_url: *const libc::c_char,
    cfg: *const TstRawSenderConfig,
    policy: *const TstReconnectPolicy,
) -> *mut TstManagedRawSender {
    crate::panic::ffi_catch(std::ptr::null_mut(), || {
        let cfg = match unsafe { cfg.as_ref() } {
            Some(c) => c.inner.clone(),
            None => tst_pipeline::RawSenderConfig::default(),
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

        let initial = match crate::sender::connect::connect_srt(&url.host, url.port, &socket_cfg) {
            Ok(t) => t,
            Err(e) => {
                record_transport_error(&e);
                return std::ptr::null_mut();
            }
        };
        let host = url.host.clone();
        let port = url.port;
        let cfg_for_reconnect = socket_cfg.clone();
        let factory = move || crate::sender::connect::connect_srt(&host, port, &cfg_for_reconnect);
        let managed = ManagedTransport::new(initial, factory, policy);
        let sender = RawSender::new(managed, cfg);
        let cancel = sender.cancel_handle();
        let was_cancelled = Arc::new(AtomicBool::new(false));
        Box::into_raw(Box::new(TstManagedRawSender {
            inner: Handle::new(sender),
            cancel,
            was_cancelled,
        }))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_managed_raw_sender_send(
    p: *mut TstManagedRawSender,
    bytes: *const u8,
    len: usize,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null sender pointer");
        return TstError::InvalidConfig as i32;
    };
    let slice = match unsafe { crate::ffi_slice::ffi_slice(bytes, len, "bytes") } {
        Ok(s) => s,
        Err(code) => return code,
    };
    handle.inner.with_inner_mut(|s| match s.send(slice) {
        Ok(()) => 0,
        Err(e) => record_shell_error(&e),
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_managed_raw_sender_close(p: *mut TstManagedRawSender) {
    crate::panic::ffi_catch((), || {
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
    });
}

/// Cancel a `tst_managed_raw_sender_t`. Same semantics as
/// `tst_raw_sender_cancel`; reaches the currently-active inner
/// transport's cancel handle through `ManagedTransport`'s atomic
/// snapshot.
///
/// Returns 0 on success, `TST_E_INVALID_CONFIG` if the pointer is null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_managed_raw_sender_cancel(p: *mut TstManagedRawSender) -> libc::c_int {
    crate::panic::ffi_catch(TstError::Internal as i32, || {
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
    })
}

/// Snapshot stats for a `tst_raw_sender_t` into `*out`.
///
/// Returns 0 on success, `TST_E_INVALID_CONFIG` if either pointer is
/// null, or `TST_E_CLOSED` if the sender has been closed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_raw_sender_get_stats(
    p: *mut TstRawSender,
    out: *mut crate::stats::TstRawSendStats,
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
        let stats = crate::stats::TstRawSendStats::from(&s.stats());
        unsafe { *out = stats };
        0
    })
}

/// Read wire-level transport stats for the underlying libsrt socket.
/// See [`tst_mux_sender_get_socket_stats`](crate::sender::mux_sender::tst_mux_sender_get_socket_stats)
/// for full semantics — same shape, different handle type.
///
/// # Safety
///
/// Caller MUST ensure `p` is a valid `*mut TstRawSender` opened via
/// `tst_raw_sender_open` and `out` points to a writable `TstSocketStats`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_raw_sender_get_socket_stats(
    p: *mut TstRawSender,
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
    // RawSender already exposes pub fn transport(&self) -> &T; reach
    // through it directly rather than adding a sibling socket_stats()
    // method on the shell.
    use tst_core::transport::Transport;
    handle
        .inner
        .with_inner_ref(|s| match s.transport().socket_stats() {
            Some(stats) => {
                unsafe { *out = (&stats).into() };
                0
            }
            None => record_not_available(
                "raw sender socket stats unavailable (transport not connected or closed)",
            ),
        })
}

/// Reset stats counters for a `tst_raw_sender_t` to zero.
///
/// Returns 0 on success, `TST_E_INVALID_CONFIG` if the pointer is
/// null, or `TST_E_CLOSED` if the sender has been closed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_raw_sender_reset_stats(p: *mut TstRawSender) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null sender pointer");
        return TstError::InvalidConfig as i32;
    };
    handle.inner.with_inner_mut(|s| {
        s.reset_stats();
        0
    })
}

/// Snapshot stats for a `tst_managed_raw_sender_t` into `*out`.
///
/// Returns 0 on success, `TST_E_INVALID_CONFIG` if either pointer is
/// null, or `TST_E_CLOSED` if the sender has been closed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_managed_raw_sender_get_stats(
    p: *mut TstManagedRawSender,
    out: *mut crate::stats::TstRawSendStats,
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
        let stats = crate::stats::TstRawSendStats::from(&s.stats());
        unsafe { *out = stats };
        0
    })
}

/// Managed sibling of [`tst_raw_sender_get_socket_stats`]. Returns
/// `TST_E_NOT_AVAILABLE` when the reconnect loop currently has no live
/// inner socket.
///
/// # Safety
///
/// Caller MUST ensure `p` is a valid `*mut TstManagedRawSender` opened
/// via `tst_managed_raw_sender_open` and `out` points to a writable
/// `TstSocketStats`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_managed_raw_sender_get_socket_stats(
    p: *mut TstManagedRawSender,
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
    use tst_core::transport::Transport;
    handle
        .inner
        .with_inner_ref(|s| match s.transport().socket_stats() {
            Some(stats) => {
                unsafe { *out = (&stats).into() };
                0
            }
            None => record_not_available(
                "raw sender socket stats unavailable (transport not connected or closed)",
            ),
        })
}

/// Reset stats counters for a `tst_managed_raw_sender_t` to zero.
///
/// Returns 0 on success, `TST_E_INVALID_CONFIG` if the pointer is
/// null, or `TST_E_CLOSED` if the sender has been closed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_managed_raw_sender_reset_stats(
    p: *mut TstManagedRawSender,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null sender pointer");
        return TstError::InvalidConfig as i32;
    };
    handle.inner.with_inner_mut(|s| {
        s.reset_stats();
        0
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_close_is_safe() {
        unsafe {
            tst_raw_sender_close(std::ptr::null_mut());
            tst_managed_raw_sender_close(std::ptr::null_mut());
        }
    }

    #[test]
    fn null_cancel_returns_invalid_config() {
        let rc = unsafe { tst_raw_sender_cancel(std::ptr::null_mut()) };
        assert_eq!(rc, TstError::InvalidConfig as i32);
    }

    #[test]
    fn managed_null_cancel_returns_invalid_config() {
        let rc = unsafe { tst_managed_raw_sender_cancel(std::ptr::null_mut()) };
        assert_eq!(rc, TstError::InvalidConfig as i32);
    }
}
