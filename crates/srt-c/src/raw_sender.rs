//! `srtc_raw_sender_t` (plain) and `srtc_managed_raw_sender_t` (managed).
//!
//! One _send call = one outbound SRT message of the exact length passed in.

use crate::config::{SrtcRawSenderConfig, SrtcReconnectPolicy};
use crate::error::{SrtcError, record_transport_error, set_last_error, srtc_get_last_error};
use crate::handle::Handle;
use crate::mux_sender::parse_c_url;
use srt_core::pipeline::{ManagedTransport, RawSender, SrtTransport};
use srt_core::srt::SocketBuilder;

fn connect_srt(host: &str, port: u16) -> Result<SrtTransport, srt_core::pipeline::TransportError> {
    let socket = SocketBuilder::new()
        .connect(format!("{host}:{port}").as_str())
        .map_err(|e| srt_core::pipeline::TransportError::Broken(format!("connect: {e}")))?;
    Ok(SrtTransport::new(socket))
}

// ------------------------------------------------------------------
// srtc_raw_sender_t
// ------------------------------------------------------------------

pub struct SrtcRawSender {
    inner: Handle<RawSender<SrtTransport>>,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_raw_sender_open(
    srt_url: *const libc::c_char,
    cfg: *const SrtcRawSenderConfig,
) -> *mut SrtcRawSender {
    let cfg = match unsafe { cfg.as_ref() } {
        Some(c) => c.inner.clone(),
        None => srt_core::pipeline::RawSenderConfig::default(),
    };
    let url = match unsafe { parse_c_url(srt_url) } {
        Ok(u) => u,
        Err(()) => return std::ptr::null_mut(),
    };
    let transport = match connect_srt(&url.host, url.port) {
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
    let url = match unsafe { parse_c_url(srt_url) } {
        Ok(u) => u,
        Err(()) => return std::ptr::null_mut(),
    };
    let initial = match connect_srt(&url.host, url.port) {
        Ok(t) => t,
        Err(e) => {
            record_transport_error(&e);
            return std::ptr::null_mut();
        }
    };
    let host = url.host.clone();
    let port = url.port;
    let factory = move || connect_srt(&host, port);
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
