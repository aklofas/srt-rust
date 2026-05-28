//! `TstUdpSender` handle type and data-path entry points.
//!
//! Open a UDP-backed raw TS byte sender with `tst_udp_sender_open`.
//! Push pre-muxed TS bytes with `tst_udp_sender_send_ts`. Free the
//! handle with `tst_udp_sender_close`.
//!
//! Pattern mirrors `crates/tst-c/src/rtp/sender.rs` exactly — error
//! mapping, `ffi_catch` wrapping, `Handle::with_inner_mut/_ref` usage,
//! and FFI slice handling are identical.
//!
//! **No cancel:** the UDP transport does not expose a `cancel_handle()`,
//! so there is no `tst_udp_sender_cancel` entry point and no cancel /
//! `was_cancelled` side-channel. `_close` simply drops the handle. To
//! unblock a thread parked in `_send_ts`, close the handle from the same
//! thread (or rely on the socket's send-side behavior).

use std::os::raw::c_char;

use tst_pipeline::{Sender, SenderConfig};
use tst_udp::{UdpTransport, UdpTransportBuilder};

use crate::error::{TstError, record_not_available, record_shell_error, set_last_error};
use crate::handle::Handle;
use crate::stats::TstSenderStats;

// ---------------------------------------------------------------------------
// Handle type
// ---------------------------------------------------------------------------

/// Opaque handle for a UDP-backed raw TS byte sender.
///
/// Returned by [`tst_udp_sender_open`]. Freed with
/// [`tst_udp_sender_close`].
pub struct TstUdpSender {
    pub(crate) inner: Handle<Sender<UdpTransport>>,
}

// ---------------------------------------------------------------------------
// Open
// ---------------------------------------------------------------------------

/// Open a UDP sender to the unicast or multicast endpoint described by
/// `url`. Returns `NULL` on error; check `tst_get_last_error()` for the
/// negative error code and `tst_get_last_error_str()` for a detail message.
///
/// URL grammar:
/// - `udp://host:port` — unicast send
/// - `udp://group:port` (group ∈ 224.0.0.0/4 or ff00::/8) — multicast send
/// - Query params: `?ttl=N`, `?iface=eth0`, `?tos=0xb8`, `?sndbuf=2M`,
///   `?pkt_size=1316`, `?localaddr=...`
///
/// # Safety
///
/// `url` must be a NUL-terminated C string valid for the duration of
/// this call. The returned handle must eventually be freed with
/// `tst_udp_sender_close`.
#[cfg(feature = "udp")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_udp_sender_open(url: *const c_char) -> *mut TstUdpSender {
    crate::panic::ffi_catch(std::ptr::null_mut(), || {
        let url_str = match unsafe { super::url::parse_url_str(url) } {
            Some(s) => s,
            None => return std::ptr::null_mut(),
        };
        let builder = match UdpTransportBuilder::from_url(url_str) {
            Ok(b) => b,
            Err(e) => {
                set_last_error(TstError::UdpConfig, &format!("udp url parse: {e}"));
                return std::ptr::null_mut();
            }
        };
        let transport = match builder.build() {
            Ok(t) => t,
            Err(e) => {
                let code = crate::error::udp_error_to_code(&e);
                set_last_error(code, &format!("udp build: {e}"));
                return std::ptr::null_mut();
            }
        };
        let sender = Sender::new(transport, SenderConfig::default());
        Box::into_raw(Box::new(TstUdpSender {
            inner: Handle::new(sender),
        }))
    })
}

// ---------------------------------------------------------------------------
// Close
// ---------------------------------------------------------------------------

/// Close and free a `tst_udp_sender_t`.
///
/// Safe to call with `NULL` (no-op). After this call the pointer is
/// invalid; passing the same non-null pointer twice is undefined
/// behavior (use-after-free on the consumed `Box`).
///
/// # Safety
///
/// `p` must be NULL or a valid non-freed `*mut TstUdpSender` returned
/// by `tst_udp_sender_open`.
#[cfg(feature = "udp")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_udp_sender_close(p: *mut TstUdpSender) {
    crate::panic::ffi_catch((), || {
        if p.is_null() {
            return;
        }
        let boxed = unsafe { Box::from_raw(p) };
        boxed.inner.close();
        drop(boxed);
    });
}

// ---------------------------------------------------------------------------
// Data-path entry points
// ---------------------------------------------------------------------------

/// Push pre-muxed TS bytes through the UDP sender.
///
/// `bytes` must point to a buffer of `len` bytes. `len` SHOULD be a
/// multiple of 188 (one or more MPEG-TS packets); the underlying
/// sender will accept any non-zero length but non-aligned buffers
/// may cause sync issues at the receiver.
///
/// Returns 0 on success, a negative `TST_E_*` code on failure.
///
/// # Safety
///
/// `p` must be a valid non-freed `*mut TstUdpSender`. `bytes` must be
/// readable for `len` bytes.
#[cfg(feature = "udp")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_udp_sender_send_ts(
    p: *mut TstUdpSender,
    bytes: *const u8,
    len: usize,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null udp sender pointer");
        return TstError::InvalidConfig as i32;
    };
    let slice = match unsafe { crate::ffi_slice::ffi_slice(bytes, len, "bytes") } {
        Ok(s) => s,
        Err(code) => return code,
    };
    handle.inner.with_inner_mut(|s| match s.send_ts(slice) {
        Ok(()) => 0,
        Err(e) => record_shell_error(&e),
    })
}

/// Snapshot stats for a `tst_udp_sender_t` into `*out`.
///
/// Returns 0 on success, `TST_E_INVALID_CONFIG` if either pointer is
/// null, or `TST_E_CLOSED` if the sender has been closed.
///
/// # Safety
///
/// `p` must be a valid `*mut TstUdpSender` opened via `tst_udp_sender_open`.
/// `out` must point to a writable `TstSenderStats`.
#[cfg(feature = "udp")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_udp_sender_get_stats(
    p: *mut TstUdpSender,
    out: *mut TstSenderStats,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null udp sender pointer");
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

/// Read wire-level transport stats for the underlying UDP socket.
///
/// `out` MUST point to a writable `TstSocketStats`; the function zeros
/// the struct on failure.
///
/// Returns 0 on success, `TST_E_INVALID_CONFIG` if either pointer is null,
/// `TST_E_NOT_AVAILABLE` if the transport has no live stats
/// (e.g., socket not yet connected or already closed), or
/// `TST_E_CLOSED` if the handle was closed.
///
/// # Safety
///
/// `p` must be a valid `*mut TstUdpSender` opened via `tst_udp_sender_open`.
/// `out` must point to a writable `TstSocketStats`.
#[cfg(feature = "udp")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_udp_sender_get_socket_stats(
    p: *mut TstUdpSender,
    out: *mut crate::stats::TstSocketStats,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null udp sender pointer");
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
        None => record_not_available(
            "udp sender socket stats unavailable (transport not connected or closed)",
        ),
    })
}

/// Reset stats counters for a `tst_udp_sender_t` to zero.
///
/// Returns 0 on success, `TST_E_INVALID_CONFIG` if the pointer is null,
/// or `TST_E_CLOSED` if the sender has been closed.
///
/// # Safety
///
/// `p` must be a valid `*mut TstUdpSender` opened via `tst_udp_sender_open`.
#[cfg(feature = "udp")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_udp_sender_reset_stats(p: *mut TstUdpSender) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null udp sender pointer");
        return TstError::InvalidConfig as i32;
    };
    handle.inner.with_inner_mut(|s| {
        s.reset_stats();
        0
    })
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_close_is_safe() {
        unsafe { tst_udp_sender_close(std::ptr::null_mut()) };
    }

    #[test]
    fn null_send_ts_returns_invalid_config() {
        let rc = unsafe { tst_udp_sender_send_ts(std::ptr::null_mut(), std::ptr::null(), 0) };
        assert_eq!(rc, TstError::InvalidConfig as i32);
    }

    #[test]
    fn null_get_stats_returns_invalid_config() {
        let mut stats = TstSenderStats::default();
        let rc = unsafe { tst_udp_sender_get_stats(std::ptr::null_mut(), &mut stats) };
        assert_eq!(rc, TstError::InvalidConfig as i32);
    }

    #[test]
    fn null_reset_stats_returns_invalid_config() {
        let rc = unsafe { tst_udp_sender_reset_stats(std::ptr::null_mut()) };
        assert_eq!(rc, TstError::InvalidConfig as i32);
    }
}
