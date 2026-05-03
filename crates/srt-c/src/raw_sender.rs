//! `srtc_raw_sender_t` (plain) and `srtc_managed_raw_sender_t` (managed).
//!
//! One _send call = one outbound SRT message of the exact length passed in.

use crate::config::{SrtcRawSenderConfig, SrtcReconnectPolicy};
use crate::error::{SrtcError, record_transport_error, set_last_error, srtc_get_last_error};
use crate::handle::Handle;
use crate::mux_sender::parse_c_srt_url;
use srt_core::pipeline::{ManagedTransport, RawSender, SrtTransport};

// ------------------------------------------------------------------
// srtc_raw_sender_t
// ------------------------------------------------------------------

pub struct SrtcRawSender {
    inner: Handle<RawSender<SrtTransport>>,
}

/// Open a `srtc_raw_sender_t` connected via SRT.
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
pub unsafe extern "C" fn srtc_raw_sender_open(
    srt_url: *const libc::c_char,
    cfg: *const SrtcRawSenderConfig,
) -> *mut SrtcRawSender {
    let cfg = match unsafe { cfg.as_ref() } {
        Some(c) => c.inner.clone(),
        None => srt_core::pipeline::RawSenderConfig::default(),
    };
    let url = match unsafe { parse_c_srt_url(srt_url) } {
        Ok(u) => u,
        Err(()) => return std::ptr::null_mut(),
    };
    let mut socket_cfg = srt_core::srt::config::SocketConfig::default();
    url.overlay.apply_to_socket(&mut socket_cfg);
    let transport = match crate::connect::connect_srt(&url.host, url.port, &socket_cfg) {
        Ok(t) => t,
        Err(e) => {
            record_transport_error(&e);
            return std::ptr::null_mut();
        }
    };
    Box::into_raw(Box::new(SrtcRawSender {
        inner: Handle::new(RawSender::new(transport, cfg)),
    }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_raw_sender_send(
    p: *mut SrtcRawSender,
    bytes: *const u8,
    len: usize,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(SrtcError::InvalidConfig, "null sender pointer");
        return SrtcError::InvalidConfig as i32;
    };
    if bytes.is_null() && len > 0 {
        set_last_error(SrtcError::InvalidConfig, "null bytes with non-zero len");
        return SrtcError::InvalidConfig as i32;
    }
    let slice = unsafe { std::slice::from_raw_parts(bytes, len) };
    handle.inner.with_inner_mut(|s| match s.send(slice) {
        Ok(()) => 0,
        Err(e) => {
            record_transport_error(&e);
            unsafe { srtc_get_last_error() }
        }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_raw_sender_close(p: *mut SrtcRawSender) {
    if p.is_null() {
        return;
    }
    let boxed = unsafe { Box::from_raw(p) };
    boxed.inner.close();
    drop(boxed);
}

// ------------------------------------------------------------------
// srtc_managed_raw_sender_t
// ------------------------------------------------------------------

pub struct SrtcManagedRawSender {
    inner: Handle<RawSender<ManagedTransport<SrtTransport>>>,
}

/// Open a `srtc_managed_raw_sender_t` connected via SRT.
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
pub unsafe extern "C" fn srtc_managed_raw_sender_open(
    srt_url: *const libc::c_char,
    cfg: *const SrtcRawSenderConfig,
    policy: *const SrtcReconnectPolicy,
) -> *mut SrtcManagedRawSender {
    let cfg = match unsafe { cfg.as_ref() } {
        Some(c) => c.inner.clone(),
        None => srt_core::pipeline::RawSenderConfig::default(),
    };
    let policy = match unsafe { policy.as_ref() } {
        Some(p) => p.inner.clone(),
        None => srt_core::pipeline::ReconnectPolicy::default(),
    };
    let url = match unsafe { parse_c_srt_url(srt_url) } {
        Ok(u) => u,
        Err(()) => return std::ptr::null_mut(),
    };
    let mut socket_cfg = srt_core::srt::config::SocketConfig::default();
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
    Box::into_raw(Box::new(SrtcManagedRawSender {
        inner: Handle::new(RawSender::new(managed, cfg)),
    }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_managed_raw_sender_send(
    p: *mut SrtcManagedRawSender,
    bytes: *const u8,
    len: usize,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(SrtcError::InvalidConfig, "null sender pointer");
        return SrtcError::InvalidConfig as i32;
    };
    if bytes.is_null() && len > 0 {
        set_last_error(SrtcError::InvalidConfig, "null bytes with non-zero len");
        return SrtcError::InvalidConfig as i32;
    }
    let slice = unsafe { std::slice::from_raw_parts(bytes, len) };
    handle.inner.with_inner_mut(|s| match s.send(slice) {
        Ok(()) => 0,
        Err(e) => {
            record_transport_error(&e);
            unsafe { srtc_get_last_error() }
        }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_managed_raw_sender_close(p: *mut SrtcManagedRawSender) {
    if p.is_null() {
        return;
    }
    let boxed = unsafe { Box::from_raw(p) };
    boxed.inner.close();
    drop(boxed);
}

/// Snapshot stats for a `srtc_raw_sender_t` into `*out`.
///
/// Returns 0 on success, `SRTC_E_INVALID_CONFIG` if either pointer is
/// null, or `SRTC_E_CLOSED` if the sender has been closed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_raw_sender_get_stats(
    p: *mut SrtcRawSender,
    out: *mut crate::stats::SrtcRawSenderStats,
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
        let stats = crate::stats::SrtcRawSenderStats::from(&s.stats());
        unsafe { *out = stats };
        0
    })
}

/// Reset stats counters for a `srtc_raw_sender_t` to zero.
///
/// Returns 0 on success, `SRTC_E_INVALID_CONFIG` if the pointer is
/// null, or `SRTC_E_CLOSED` if the sender has been closed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_raw_sender_reset_stats(p: *mut SrtcRawSender) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(SrtcError::InvalidConfig, "null sender pointer");
        return SrtcError::InvalidConfig as i32;
    };
    handle.inner.with_inner_mut(|s| {
        s.reset_stats();
        0
    })
}

/// Snapshot stats for a `srtc_managed_raw_sender_t` into `*out`.
///
/// Returns 0 on success, `SRTC_E_INVALID_CONFIG` if either pointer is
/// null, or `SRTC_E_CLOSED` if the sender has been closed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_managed_raw_sender_get_stats(
    p: *mut SrtcManagedRawSender,
    out: *mut crate::stats::SrtcRawSenderStats,
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
        let stats = crate::stats::SrtcRawSenderStats::from(&s.stats());
        unsafe { *out = stats };
        0
    })
}

/// Reset stats counters for a `srtc_managed_raw_sender_t` to zero.
///
/// Returns 0 on success, `SRTC_E_INVALID_CONFIG` if the pointer is
/// null, or `SRTC_E_CLOSED` if the sender has been closed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_managed_raw_sender_reset_stats(
    p: *mut SrtcManagedRawSender,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(SrtcError::InvalidConfig, "null sender pointer");
        return SrtcError::InvalidConfig as i32;
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
            srtc_raw_sender_close(std::ptr::null_mut());
            srtc_managed_raw_sender_close(std::ptr::null_mut());
        }
    }
}
