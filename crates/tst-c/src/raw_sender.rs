//! `tst_raw_sender_t` (plain) and `tst_managed_raw_sender_t` (managed).
//!
//! One _send call = one outbound SRT message of the exact length passed in.

use crate::config::{TstRawSenderConfig, TstReconnectPolicy};
use crate::error::{TstError, record_transport_error, set_last_error, tst_get_last_error};
use crate::handle::Handle;
use crate::mux_sender::parse_c_srt_url;
use tst_pipeline::{ManagedTransport, RawSender};
use tst_srt::SrtTransport;

// ------------------------------------------------------------------
// tst_raw_sender_t
// ------------------------------------------------------------------

pub struct TstRawSender {
    inner: Handle<RawSender<SrtTransport>>,
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
        let transport = match crate::connect::connect_srt(&url.host, url.port, &socket_cfg) {
            Ok(t) => t,
            Err(e) => {
                record_transport_error(&e);
                return std::ptr::null_mut();
            }
        };
        Box::into_raw(Box::new(TstRawSender {
            inner: Handle::new(RawSender::new(transport, cfg)),
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
    if bytes.is_null() && len > 0 {
        set_last_error(TstError::InvalidConfig, "null bytes with non-zero len");
        return TstError::InvalidConfig as i32;
    }
    let slice = unsafe { std::slice::from_raw_parts(bytes, len) };
    handle.inner.with_inner_mut(|s| match s.send(slice) {
        Ok(()) => 0,
        Err(e) => {
            record_transport_error(&e);
            unsafe { tst_get_last_error() }
        }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_raw_sender_close(p: *mut TstRawSender) {
    if p.is_null() {
        return;
    }
    let boxed = unsafe { Box::from_raw(p) };
    boxed.inner.close();
    drop(boxed);
}

// ------------------------------------------------------------------
// tst_managed_raw_sender_t
// ------------------------------------------------------------------

pub struct TstManagedRawSender {
    inner: Handle<RawSender<ManagedTransport<SrtTransport>>>,
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
        Box::into_raw(Box::new(TstManagedRawSender {
            inner: Handle::new(RawSender::new(managed, cfg)),
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
    if bytes.is_null() && len > 0 {
        set_last_error(TstError::InvalidConfig, "null bytes with non-zero len");
        return TstError::InvalidConfig as i32;
    }
    let slice = unsafe { std::slice::from_raw_parts(bytes, len) };
    handle.inner.with_inner_mut(|s| match s.send(slice) {
        Ok(()) => 0,
        Err(e) => {
            record_transport_error(&e);
            unsafe { tst_get_last_error() }
        }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_managed_raw_sender_close(p: *mut TstManagedRawSender) {
    if p.is_null() {
        return;
    }
    let boxed = unsafe { Box::from_raw(p) };
    boxed.inner.close();
    drop(boxed);
}

/// Snapshot stats for a `tst_raw_sender_t` into `*out`.
///
/// Returns 0 on success, `TST_E_INVALID_CONFIG` if either pointer is
/// null, or `TST_E_CLOSED` if the sender has been closed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_raw_sender_get_stats(
    p: *mut TstRawSender,
    out: *mut crate::stats::TstRawSenderStats,
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
        let stats = crate::stats::TstRawSenderStats::from(&s.stats());
        unsafe { *out = stats };
        0
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
    out: *mut crate::stats::TstRawSenderStats,
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
        let stats = crate::stats::TstRawSenderStats::from(&s.stats());
        unsafe { *out = stats };
        0
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
}
