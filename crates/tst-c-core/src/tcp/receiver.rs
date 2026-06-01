//! `TstTcpReceiver` handle type and data-path entry points.
//!
//! Open a TCP-backed raw TS byte receiver with `tst_tcp_recv_open`.
//! Pull 188-byte MPEG-TS packets one at a time with
//! `tst_tcp_receiver_recv_ts`. Free the handle with
//! `tst_tcp_receiver_close`.
//!
//! Pattern mirrors `crates/tst-c/src/udp/receiver.rs` — error mapping,
//! `ffi_catch` wrapping, and `Handle::with_inner_mut/_ref` usage are
//! identical, except the transport type is `TcpTransport` not
//! `UdpRecvTransport`.
//!
//! **Single transport type:** TCP uses one `TcpTransport` that implements
//! both `Transport` and `RecvTransport`. `Receiver<TcpTransport>` uses the
//! `RecvTransport` side. Construction uses `TcpTransportBuilder::from_url`
//! (same as the sender path — role is determined by the pipeline shell).
//!
//! **No cancel:** the TCP transport does not expose a `cancel_handle()`,
//! so there is no `tst_tcp_receiver_cancel` entry point. `_close` simply
//! drops the handle. Without a caller-cancel path, a graceful transport
//! close maps to `TST_E_END_OF_STREAM`.

use std::os::raw::c_char;

use tst_core::mpegts::common::TS_PACKET_SIZE;
use tst_pipeline::{Receiver, ReceiverConfig, ShellErrorKind};
use tst_tcp::{TcpTransport, TcpTransportBuilder};

use crate::error::{
    TstError, record_eos, record_not_available, record_shell_error, set_last_error,
};
use crate::handle::Handle;
use crate::stats::TstReceiverStats;

// ---------------------------------------------------------------------------
// Handle type
// ---------------------------------------------------------------------------

/// Opaque handle for a TCP-backed raw TS byte receiver.
///
/// Returned by [`tst_tcp_recv_open`]. Freed with
/// [`tst_tcp_receiver_close`].
pub struct TstTcpReceiver {
    pub(crate) inner: Handle<Receiver<TcpTransport>>,
}

// ---------------------------------------------------------------------------
// Open
// ---------------------------------------------------------------------------

/// Open a TCP receiver connecting to the endpoint described by `url`.
/// Returns `NULL` on error.
///
/// URL grammar:
/// - `tcp://host:port` — connect to a plain TCP listener
/// - `tcps://host:port` — connect with TLS (disabled if built without `tls` feature)
/// - Query params: `?nodelay=1`, `?rcvbuf=N`, `?sndbuf=N`, `?pkt_size=N`,
///   `?connect_timeout=Ns`
///
/// For a listener-accepted connection, use `tst_tcp_listener_accept_receiver`
/// instead.
///
/// # Safety
///
/// `url` must be a NUL-terminated C string. The returned handle must
/// eventually be freed with `tst_tcp_receiver_close`.
#[cfg(feature = "tcp")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_tcp_recv_open(url: *const c_char) -> *mut TstTcpReceiver {
    crate::panic::ffi_catch(std::ptr::null_mut(), || {
        let url_str = match unsafe { super::url::parse_url_str(url) } {
            Some(s) => s,
            None => return std::ptr::null_mut(),
        };
        let builder = match TcpTransportBuilder::from_url(url_str) {
            Ok(b) => b,
            Err(e) => {
                set_last_error(TstError::TcpConfig, &format!("tcp url parse: {e}"));
                return std::ptr::null_mut();
            }
        };
        let transport = match builder.build() {
            Ok(t) => t,
            Err(e) => {
                let code = crate::error::tcp_error_to_code(&e);
                set_last_error(code, &format!("tcp connect: {e}"));
                return std::ptr::null_mut();
            }
        };
        let receiver = Receiver::new(transport, ReceiverConfig::default());
        Box::into_raw(Box::new(TstTcpReceiver {
            inner: Handle::new(receiver),
        }))
    })
}

// ---------------------------------------------------------------------------
// Close
// ---------------------------------------------------------------------------

/// Close and free a `tst_tcp_receiver_t`.
///
/// Safe to call with `NULL` (no-op). See `tst_tcp_sender_close` for
/// the ownership semantics.
///
/// # Safety
///
/// `p` must be NULL or a valid non-freed `*mut TstTcpReceiver` returned
/// by `tst_tcp_recv_open` or `tst_tcp_listener_accept_receiver`.
#[cfg(feature = "tcp")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_tcp_receiver_close(p: *mut TstTcpReceiver) {
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

/// Block until one 188-byte MPEG-TS packet is ready, then copy it
/// into the caller's buffer.
///
/// `buf` MUST point to a buffer of at least `buf_len` bytes (at least
/// 188 bytes). On success, `*out_n` is set to the number of bytes
/// written (always 188). On failure the contents of `buf` are
/// unspecified.
///
/// TCP is a reliable bytestream: the library accumulates incoming bytes
/// until it has a complete 188-byte TS packet (synchronised on the 0x47
/// sync byte). Backpressure is handled by the OS TCP flow-control window —
/// this call blocks until the receiver buffer fills or the peer closes.
///
/// Returns:
/// - `0` on success (188 bytes written to `buf`, `*out_n` = 188)
/// - `TST_E_END_OF_STREAM` (-12) on graceful peer close / EOF
/// - `TST_E_CLOSED` (-7) if the handle was `_close`'d
/// - `TST_E_TRANSPORT` (-8) on transport failure
/// - `TST_E_INVALID_CONFIG` (-1) on null pointer arguments or too-small buffer
///
/// # Safety
///
/// `p` must be a valid non-freed `*mut TstTcpReceiver`. `buf` must be
/// writable for `buf_len` bytes. `out_n` must be a valid `*mut usize`.
#[cfg(feature = "tcp")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_tcp_receiver_recv_ts(
    p: *mut TstTcpReceiver,
    buf: *mut u8,
    buf_len: usize,
    out_n: *mut usize,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null tcp receiver pointer");
        return TstError::InvalidConfig as i32;
    };
    if buf.is_null() {
        set_last_error(TstError::InvalidConfig, "null buf pointer");
        return TstError::InvalidConfig as i32;
    }
    if out_n.is_null() {
        set_last_error(TstError::InvalidConfig, "null out_n pointer");
        return TstError::InvalidConfig as i32;
    }
    if buf_len < TS_PACKET_SIZE {
        set_last_error(
            TstError::InvalidConfig,
            &format!("buf_len {buf_len} too small (need at least {TS_PACKET_SIZE})"),
        );
        return TstError::InvalidConfig as i32;
    }
    handle.inner.with_inner_mut(|rx| match rx.next_packet() {
        Ok(pkt) => {
            // SAFETY: buf non-null + writable for >= TS_PACKET_SIZE bytes per guard.
            unsafe {
                std::ptr::copy_nonoverlapping(pkt.as_ptr(), buf, TS_PACKET_SIZE);
                *out_n = TS_PACKET_SIZE;
            }
            0
        }
        // No caller-cancel side-channel on TCP — a Closed or peer-Broken
        // condition means the stream ended; map to EOS.
        Err(e) if e.kind == ShellErrorKind::Closed || e.kind == ShellErrorKind::TransportBroken => {
            record_eos();
            TstError::EndOfStream as i32
        }
        Err(e) => record_shell_error(&e),
    })
}

/// Snapshot stats for a `tst_tcp_receiver_t` into `*out`.
///
/// Returns 0 on success, `TST_E_INVALID_CONFIG` if either pointer is
/// null, or `TST_E_CLOSED` if the receiver has been closed.
///
/// # Safety
///
/// `p` must be a valid `*mut TstTcpReceiver` opened via `tst_tcp_recv_open`
/// or `tst_tcp_listener_accept_receiver`.
/// `out` must point to a writable `TstReceiverStats`.
#[cfg(feature = "tcp")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_tcp_receiver_get_stats(
    p: *mut TstTcpReceiver,
    out: *mut TstReceiverStats,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null tcp receiver pointer");
        return TstError::InvalidConfig as i32;
    };
    if out.is_null() {
        set_last_error(TstError::InvalidConfig, "null out pointer");
        return TstError::InvalidConfig as i32;
    }
    handle.inner.with_inner_ref(|rx| {
        let stats = TstReceiverStats::from(&rx.stats());
        unsafe { *out = stats };
        0
    })
}

/// Read wire-level transport stats for the underlying TCP socket.
///
/// `out` MUST point to a writable `TstSocketStats`; the function zeros
/// the struct on failure.
///
/// Returns 0 on success, `TST_E_INVALID_CONFIG` if either pointer is null,
/// `TST_E_NOT_AVAILABLE` if no live socket stats are available, or
/// `TST_E_CLOSED` if the handle was closed.
///
/// # Safety
///
/// `p` must be a valid `*mut TstTcpReceiver` opened via `tst_tcp_recv_open`
/// or `tst_tcp_listener_accept_receiver`.
/// `out` must point to a writable `TstSocketStats`.
#[cfg(feature = "tcp")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_tcp_receiver_get_socket_stats(
    p: *mut TstTcpReceiver,
    out: *mut crate::stats::TstSocketStats,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null tcp receiver pointer");
        return TstError::InvalidConfig as i32;
    };
    if out.is_null() {
        set_last_error(TstError::InvalidConfig, "null out pointer");
        return TstError::InvalidConfig as i32;
    }
    unsafe { *out = crate::stats::TstSocketStats::default() };
    handle.inner.with_inner_ref(|rx| match rx.socket_stats() {
        Some(stats) => {
            unsafe { *out = (&stats).into() };
            0
        }
        None => record_not_available(
            "tcp receiver socket stats unavailable (transport not connected or closed)",
        ),
    })
}

/// Reset stats counters for a `tst_tcp_receiver_t` to zero.
///
/// Returns 0 on success, `TST_E_INVALID_CONFIG` if the pointer is null,
/// or `TST_E_CLOSED` if the receiver has been closed.
///
/// # Safety
///
/// `p` must be a valid `*mut TstTcpReceiver` opened via `tst_tcp_recv_open`
/// or `tst_tcp_listener_accept_receiver`.
#[cfg(feature = "tcp")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_tcp_receiver_reset_stats(p: *mut TstTcpReceiver) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null tcp receiver pointer");
        return TstError::InvalidConfig as i32;
    };
    handle.inner.with_inner_mut(|rx| {
        rx.reset_stats();
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
        unsafe { tst_tcp_receiver_close(std::ptr::null_mut()) };
    }

    #[test]
    fn null_recv_ts_returns_invalid_config() {
        let mut buf = [0u8; 188];
        let mut n: usize = 0;
        let rc = unsafe {
            tst_tcp_receiver_recv_ts(std::ptr::null_mut(), buf.as_mut_ptr(), buf.len(), &mut n)
        };
        assert_eq!(rc, TstError::InvalidConfig as i32);
    }

    #[test]
    fn null_get_stats_returns_invalid_config() {
        let mut stats = TstReceiverStats::default();
        let rc = unsafe { tst_tcp_receiver_get_stats(std::ptr::null_mut(), &mut stats) };
        assert_eq!(rc, TstError::InvalidConfig as i32);
    }

    #[test]
    fn null_reset_stats_returns_invalid_config() {
        let rc = unsafe { tst_tcp_receiver_reset_stats(std::ptr::null_mut()) };
        assert_eq!(rc, TstError::InvalidConfig as i32);
    }

    #[test]
    fn small_buf_returns_invalid_config() {
        // No live server needed — we just test the guard path.
        let mut buf = [0u8; 100]; // too small for 188-byte packet
        let mut n: usize = 0;
        let rc = unsafe {
            tst_tcp_receiver_recv_ts(std::ptr::null_mut(), buf.as_mut_ptr(), buf.len(), &mut n)
        };
        assert_eq!(rc, TstError::InvalidConfig as i32);
    }
}
