//! `srtc_ts_sender_t` and `srtc_managed_ts_sender_t`.
//!
//! Pre-muxed TS bytes -> SRT, with sync-byte framing/recovery (RECOVER or
//! STRICT mode per `srtc_ts_sender_config_t::framing_mode`).

use crate::config::{SrtcReconnectPolicy, SrtcTsSenderConfig};
use crate::error::{
    SrtcError, record_transport_error, record_ts_sender_error, set_last_error, srtc_get_last_error,
};
use crate::handle::Handle;
use crate::mux_sender::parse_c_srt_url;
use srt_core::pipeline::{ManagedTransport, SrtTransport, TsSender, TsSenderStats};

/// Public-ABI mirror of `srt_core::pipeline::TsSenderStats`. Same fields,
/// same units. Caller passes a pointer to a stack-allocated struct;
/// `srtc_ts_sender_get_stats` fills it in.
#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct SrtcTsSenderStats {
    pub bytes_pushed: u64,
    pub bytes_skipped_for_sync: u64,
    pub resync_events: u64,
    pub packets_sent: u64,
}

impl From<&TsSenderStats> for SrtcTsSenderStats {
    fn from(s: &TsSenderStats) -> Self {
        Self {
            bytes_pushed: s.bytes_pushed,
            bytes_skipped_for_sync: s.bytes_skipped_for_sync,
            resync_events: s.resync_events,
            packets_sent: s.packets_sent,
        }
    }
}

// ------------------------------------------------------------------
// srtc_ts_sender_t (plain L1)
// ------------------------------------------------------------------

pub struct SrtcTsSender {
    inner: Handle<TsSender<SrtTransport>>,
}

/// Open a `srtc_ts_sender_t` connected via SRT.
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
pub unsafe extern "C" fn srtc_ts_sender_open(
    srt_url: *const libc::c_char,
    cfg: *const SrtcTsSenderConfig,
) -> *mut SrtcTsSender {
    let cfg = match unsafe { cfg.as_ref() } {
        Some(c) => c.inner.clone(),
        None => srt_core::pipeline::TsSenderConfig::default(),
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
    Box::into_raw(Box::new(SrtcTsSender {
        inner: Handle::new(TsSender::new(transport, cfg)),
    }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_ts_sender_send_ts(
    p: *mut SrtcTsSender,
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
    handle.inner.with_inner_mut(|s| match s.send_ts(slice) {
        Ok(()) => 0,
        Err(e) => {
            record_ts_sender_error(&e);
            unsafe { srtc_get_last_error() }
        }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_ts_sender_flush(p: *mut SrtcTsSender) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(SrtcError::InvalidConfig, "null sender pointer");
        return SrtcError::InvalidConfig as i32;
    };
    handle.inner.with_inner_mut(|s| match s.flush() {
        Ok(()) => 0,
        Err(e) => {
            record_ts_sender_error(&e);
            unsafe { srtc_get_last_error() }
        }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_ts_sender_get_stats(
    p: *mut SrtcTsSender,
    out: *mut SrtcTsSenderStats,
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
        let stats = SrtcTsSenderStats::from(s.stats());
        unsafe { *out = stats };
        0
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_ts_sender_reset_stats(p: *mut SrtcTsSender) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(SrtcError::InvalidConfig, "null sender pointer");
        return SrtcError::InvalidConfig as i32;
    };
    handle.inner.with_inner_mut(|s| {
        s.reset_stats();
        0
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_ts_sender_close(p: *mut SrtcTsSender) {
    if p.is_null() {
        return;
    }
    let boxed = unsafe { Box::from_raw(p) };
    boxed.inner.close();
    drop(boxed);
}

// ------------------------------------------------------------------
// srtc_managed_ts_sender_t (managed L2)
// ------------------------------------------------------------------

pub struct SrtcManagedTsSender {
    inner: Handle<TsSender<ManagedTransport<SrtTransport>>>,
}

/// Open a `srtc_managed_ts_sender_t` connected via SRT.
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
pub unsafe extern "C" fn srtc_managed_ts_sender_open(
    srt_url: *const libc::c_char,
    cfg: *const SrtcTsSenderConfig,
    policy: *const SrtcReconnectPolicy,
) -> *mut SrtcManagedTsSender {
    let cfg = match unsafe { cfg.as_ref() } {
        Some(c) => c.inner.clone(),
        None => srt_core::pipeline::TsSenderConfig::default(),
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
    Box::into_raw(Box::new(SrtcManagedTsSender {
        inner: Handle::new(TsSender::new(managed, cfg)),
    }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_managed_ts_sender_send_ts(
    p: *mut SrtcManagedTsSender,
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
    handle.inner.with_inner_mut(|s| match s.send_ts(slice) {
        Ok(()) => 0,
        Err(e) => {
            record_ts_sender_error(&e);
            unsafe { srtc_get_last_error() }
        }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_managed_ts_sender_flush(p: *mut SrtcManagedTsSender) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(SrtcError::InvalidConfig, "null sender pointer");
        return SrtcError::InvalidConfig as i32;
    };
    handle.inner.with_inner_mut(|s| match s.flush() {
        Ok(()) => 0,
        Err(e) => {
            record_ts_sender_error(&e);
            unsafe { srtc_get_last_error() }
        }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_managed_ts_sender_get_stats(
    p: *mut SrtcManagedTsSender,
    out: *mut SrtcTsSenderStats,
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
        let stats = SrtcTsSenderStats::from(s.stats());
        unsafe { *out = stats };
        0
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_managed_ts_sender_reset_stats(
    p: *mut SrtcManagedTsSender,
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_managed_ts_sender_close(p: *mut SrtcManagedTsSender) {
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
    fn open_with_null_url_returns_null() {
        unsafe {
            let cfg = srtc_ts_sender_config_new();
            let p = srtc_ts_sender_open(std::ptr::null(), cfg);
            assert!(p.is_null());
            srtc_ts_sender_config_free(cfg);
        }
    }

    #[test]
    fn null_close_is_safe() {
        unsafe {
            srtc_ts_sender_close(std::ptr::null_mut());
            srtc_managed_ts_sender_close(std::ptr::null_mut());
        }
    }
}
