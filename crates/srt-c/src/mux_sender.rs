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
use crate::handle::Handle;
use srt_core::pipeline::{ManagedTransport, Sender, SrtTransport, TransportError};
use srt_core::srt::SocketBuilder;
use srt_core::srt::config::SocketConfig;

/// Build a fresh `SrtTransport` connected to `host:port`. Used by the
/// managed sender's reconnect closure (plain sender uses
/// `crate::connect::connect_srt` with a full `SocketConfig` instead).
fn connect_srt(host: &str, port: u16) -> Result<SrtTransport, TransportError> {
    let socket = SocketBuilder::new()
        .connect(format!("{host}:{port}").as_str())
        .map_err(|e| TransportError::Broken(format!("connect: {e}")))?;
    Ok(SrtTransport::new(socket))
}

// ------------------------------------------------------------------
// srtc_mux_sender_t (plain L1)
// ------------------------------------------------------------------

pub struct SrtcMuxSender {
    inner: Handle<Sender<SrtTransport>>,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_mux_sender_open(
    srt_url: *const libc::c_char,
    cfg: *const SrtcMuxConfig,
) -> *mut SrtcMuxSender {
    let Some(cfg) = (unsafe { cfg.as_ref() }) else {
        set_last_error(SrtcError::InvalidConfig, "null config pointer");
        return std::ptr::null_mut();
    };
    let url = match unsafe { parse_c_srt_url(srt_url) } {
        Ok(u) => u,
        Err(()) => return std::ptr::null_mut(),
    };
    let built = match cfg.builder.clone().build() {
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_managed_mux_sender_open(
    srt_url: *const libc::c_char,
    cfg: *const SrtcMuxConfig,
    policy: *const SrtcReconnectPolicy,
) -> *mut SrtcManagedMuxSender {
    let Some(cfg) = (unsafe { cfg.as_ref() }) else {
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
    let built = match cfg.builder.clone().build() {
        Ok(c) => c,
        Err(e) => {
            record_mux_error(&e);
            return std::ptr::null_mut();
        }
    };

    // Initial connect.
    let initial = match connect_srt(&url.host, url.port) {
        Ok(t) => t,
        Err(e) => {
            crate::error::record_transport_error(&e);
            return std::ptr::null_mut();
        }
    };

    // Factory closure: captures host (String) + port (u16) by move.
    let host = url.host;
    let port = url.port;
    let factory = move || connect_srt(&host, port);

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
            srtc_mux_config_add_video(cfg, 0x1011, SrtcVideoCodec::H264);
            srtc_mux_config_add_klv(cfg, 0x1031, SrtcKlvStreamType::PrivateData, false);
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
            srtc_mux_config_add_video(cfg, 0x1011, SrtcVideoCodec::H264);
            srtc_mux_config_add_klv(cfg, 0x1031, SrtcKlvStreamType::PrivateData, false);
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
