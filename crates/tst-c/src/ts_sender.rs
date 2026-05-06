//! `tst_sender_t` and `tst_managed_sender_t`.
//!
//! Pre-muxed TS bytes -> SRT, with sync-byte framing/recovery (RECOVER or
//! STRICT mode per `tst_sender_config_t::framing_mode`).

use crate::config::{TstReconnectPolicy, TstSenderConfig};
use crate::error::{
    TstError, record_transport_error, record_ts_sender_error, set_last_error, tst_get_last_error,
};
use crate::handle::Handle;
use crate::mux_sender::parse_c_srt_url;
use tst_pipeline::{ManagedTransport, Sender, SenderStats};
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
    Box::into_raw(Box::new(TstSender {
        inner: Handle::new(Sender::new(transport, cfg)),
    }))
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
        Err(e) => {
            record_ts_sender_error(&e);
            unsafe { tst_get_last_error() }
        }
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
        Err(e) => {
            record_ts_sender_error(&e);
            unsafe { tst_get_last_error() }
        }
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
        let stats = TstSenderStats::from(s.stats());
        unsafe { *out = stats };
        0
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
    boxed.inner.close();
    drop(boxed);
}

// ------------------------------------------------------------------
// tst_managed_sender_t (managed L2)
// ------------------------------------------------------------------

pub struct TstManagedSender {
    inner: Handle<Sender<ManagedTransport<SrtTransport>>>,
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
    Box::into_raw(Box::new(TstManagedSender {
        inner: Handle::new(Sender::new(managed, cfg)),
    }))
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
        Err(e) => {
            record_ts_sender_error(&e);
            unsafe { tst_get_last_error() }
        }
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
        Err(e) => {
            record_ts_sender_error(&e);
            unsafe { tst_get_last_error() }
        }
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
        let stats = TstSenderStats::from(s.stats());
        unsafe { *out = stats };
        0
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_managed_sender_reset_stats(
    p: *mut TstManagedSender,
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_managed_sender_close(p: *mut TstManagedSender) {
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
}
