//! `TstRtpReceiver` handle type and data-path entry points.
//!
//! Open an RTP-backed raw TS byte receiver with `tst_rtp_recv_open`.
//! Pull 188-byte MPEG-TS packets one at a time with
//! `tst_rtp_receiver_recv_ts`. Cancel a blocked receive with
//! `tst_rtp_receiver_cancel`. Free the handle with
//! `tst_rtp_receiver_close`.
//!
//! Pattern mirrors `crates/tst-c/src/receiver/ts_receiver.rs` exactly —
//! error mapping, `ffi_catch` wrapping, `Handle::with_inner_mut/_ref`
//! usage, the `was_cancelled` + cancel side-channel, and the
//! `ShellErrorKind::Closed` → `TST_E_CLOSED` vs `TST_E_END_OF_STREAM`
//! discrimination.

use std::os::raw::c_char;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tst_core::RecvTransport;
use tst_core::mpegts::common::TS_PACKET_SIZE;
use tst_pipeline::{Receiver, ReceiverConfig, ShellErrorKind, TransportCancel};
use tst_rtp::{RtpRecvSocketBuilder, RtpRecvTransport};

use crate::error::{
    TstError, record_eos, record_not_available, record_shell_error, set_last_error,
};
use crate::handle::Handle;
use crate::stats::TstReceiverStats;

// ---------------------------------------------------------------------------
// Handle type
// ---------------------------------------------------------------------------

/// Opaque handle for an RTP-backed raw TS byte receiver.
///
/// Returned by [`tst_rtp_recv_open`]. Freed with
/// [`tst_rtp_receiver_close`].
pub struct TstRtpReceiver {
    pub(crate) inner: Handle<Receiver<RtpRecvTransport>>,
    /// Cancel handle snapshotted at `_open` time. Reaches the underlying
    /// RTP socket so a blocked `_recv_ts` returns without waiting on the
    /// handle's `Mutex`.
    pub(crate) cancel: Option<Arc<dyn TransportCancel + Send + Sync>>,
    /// Set by `_cancel` and `_close` so the recv path can distinguish
    /// caller-initiated shutdown (`TST_E_CLOSED`) from peer EOF
    /// (`TST_E_END_OF_STREAM`).
    pub(crate) was_cancelled: Arc<AtomicBool>,
}

// ---------------------------------------------------------------------------
// Open
// ---------------------------------------------------------------------------

/// Open an RTP receiver listening on the unicast or multicast endpoint
/// described by `url`. Returns `NULL` on error.
///
/// For unicast, pass `rtp://0.0.0.0:port` or `rtp://127.0.0.1:port`
/// (host is the bind address). For multicast, pass the group address
/// (`rtp://239.0.0.1:port?iface=eth0`); the socket joins the group on
/// `iface` (or the OS-default interface when absent).
///
/// Port `0` causes the kernel to assign an ephemeral port.
///
/// # Safety
///
/// `url` must be a NUL-terminated C string. The returned handle must
/// eventually be freed with `tst_rtp_receiver_close`.
#[cfg(feature = "rtp")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_rtp_recv_open(url: *const c_char) -> *mut TstRtpReceiver {
    crate::panic::ffi_catch(std::ptr::null_mut(), || {
        let rtp_url = match unsafe { super::url::parse_url(url) } {
            Some(u) => u,
            None => return std::ptr::null_mut(),
        };
        let mut builder = RtpRecvSocketBuilder::new(rtp_url.host.clone(), rtp_url.port);
        if let Some(ref iface) = rtp_url.iface {
            builder.iface(iface.clone());
        }
        builder.pkt_size(rtp_url.pkt_size);
        let transport = match builder.listen() {
            Ok(t) => t,
            Err(e) => {
                set_last_error(TstError::RtpTransport, &format!("rtp listen: {e}"));
                return std::ptr::null_mut();
            }
        };
        let cancel = transport.cancel_handle();
        let receiver = Receiver::new(transport, ReceiverConfig::default());
        Box::into_raw(Box::new(TstRtpReceiver {
            inner: Handle::new(receiver),
            cancel,
            was_cancelled: Arc::new(AtomicBool::new(false)),
        }))
    })
}

// ---------------------------------------------------------------------------
// Close
// ---------------------------------------------------------------------------

/// Close and free a `tst_rtp_receiver_t`.
///
/// Safe to call with `NULL` (no-op). See `tst_rtp_sender_close` for
/// the ownership semantics.
///
/// # Safety
///
/// `p` must be NULL or a valid non-freed `*mut TstRtpReceiver` returned
/// by `tst_rtp_recv_open`.
#[cfg(feature = "rtp")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_rtp_receiver_close(p: *mut TstRtpReceiver) {
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
/// Returns:
/// - `0` on success (188 bytes written to `buf`, `*out_n` = 188)
/// - `TST_E_END_OF_STREAM` (-12) on graceful peer close / EOF
/// - `TST_E_CLOSED` (-7) if the handle was `_cancel`'d or `_close`'d
/// - `TST_E_TRANSPORT` (-8) on transport failure
/// - `TST_E_INVALID_CONFIG` (-1) on null pointer arguments or too-small buffer
///
/// # Safety
///
/// `p` must be a valid non-freed `*mut TstRtpReceiver`. `buf` must be
/// writable for `buf_len` bytes. `out_n` must be a valid `*mut usize`.
#[cfg(feature = "rtp")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_rtp_receiver_recv_ts(
    p: *mut TstRtpReceiver,
    buf: *mut u8,
    buf_len: usize,
    out_n: *mut usize,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null rtp receiver pointer");
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
    let was_cancelled = handle.was_cancelled.clone();
    handle.inner.with_inner_mut(|rx| match rx.next_packet() {
        Ok(pkt) => {
            // SAFETY: buf non-null + writable for >= TS_PACKET_SIZE bytes per guard.
            unsafe {
                std::ptr::copy_nonoverlapping(pkt.as_ptr(), buf, TS_PACKET_SIZE);
                *out_n = TS_PACKET_SIZE;
            }
            0
        }
        Err(e) if e.kind == ShellErrorKind::Closed => {
            if was_cancelled.load(Ordering::Acquire) {
                set_last_error(
                    TstError::Closed,
                    "rtp receiver was cancelled or closed by caller",
                );
                TstError::Closed as i32
            } else {
                record_eos();
                TstError::EndOfStream as i32
            }
        }
        // Broken on a non-cancelled handle means the peer closed — map to EOS.
        Err(e)
            if e.kind == ShellErrorKind::TransportBroken
                && !was_cancelled.load(Ordering::Acquire) =>
        {
            record_eos();
            TstError::EndOfStream as i32
        }
        Err(e) => record_shell_error(&e),
    })
}

/// Cancel a `tst_rtp_receiver_t`. Signals the underlying RTP socket to
/// stop, unblocking any thread parked in `_recv_ts`. Safe to call from
/// any thread. Idempotent.
///
/// Returns 0 on success, `TST_E_INVALID_CONFIG` if the pointer is null.
///
/// After cancel, `_recv_ts` returns `TST_E_CLOSED` (not
/// `TST_E_END_OF_STREAM`). The handle must still be `_close`'d to free.
///
/// # Safety
///
/// `p` must be NULL or a valid non-freed `*mut TstRtpReceiver`.
#[cfg(feature = "rtp")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_rtp_receiver_cancel(p: *mut TstRtpReceiver) -> libc::c_int {
    crate::panic::ffi_catch(TstError::Internal as i32, || {
        let Some(handle) = (unsafe { p.as_ref() }) else {
            set_last_error(TstError::InvalidConfig, "null rtp receiver pointer");
            return TstError::InvalidConfig as i32;
        };
        // Side-channel: do NOT acquire handle.inner's Mutex (a concurrent
        // recv_ts holds it). The was_cancelled flag + cancel-handle Arc are
        // accessible without locking.
        handle.was_cancelled.store(true, Ordering::Release);
        if let Some(c) = &handle.cancel {
            c.cancel();
        }
        0
    })
}

/// Snapshot stats for a `tst_rtp_receiver_t` into `*out`.
///
/// Returns 0 on success, `TST_E_INVALID_CONFIG` if either pointer is
/// null, or `TST_E_CLOSED` if the receiver has been closed.
///
/// # Safety
///
/// `p` must be a valid `*mut TstRtpReceiver` opened via `tst_rtp_recv_open`.
/// `out` must point to a writable `TstReceiverStats`.
#[cfg(feature = "rtp")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_rtp_receiver_get_stats(
    p: *mut TstRtpReceiver,
    out: *mut TstReceiverStats,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null rtp receiver pointer");
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

/// Read wire-level transport stats for the underlying RTP socket.
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
/// `p` must be a valid `*mut TstRtpReceiver` opened via `tst_rtp_recv_open`.
/// `out` must point to a writable `TstSocketStats`.
#[cfg(feature = "rtp")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_rtp_receiver_get_socket_stats(
    p: *mut TstRtpReceiver,
    out: *mut crate::stats::TstSocketStats,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null rtp receiver pointer");
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
            "rtp receiver socket stats unavailable (transport not connected or closed)",
        ),
    })
}

/// Reset stats counters for a `tst_rtp_receiver_t` to zero.
///
/// Returns 0 on success, `TST_E_INVALID_CONFIG` if the pointer is null,
/// or `TST_E_CLOSED` if the receiver has been closed.
///
/// # Safety
///
/// `p` must be a valid `*mut TstRtpReceiver` opened via `tst_rtp_recv_open`.
#[cfg(feature = "rtp")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_rtp_receiver_reset_stats(p: *mut TstRtpReceiver) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null rtp receiver pointer");
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
        unsafe { tst_rtp_receiver_close(std::ptr::null_mut()) };
    }

    #[test]
    fn null_cancel_returns_invalid_config() {
        let rc = unsafe { tst_rtp_receiver_cancel(std::ptr::null_mut()) };
        assert_eq!(rc, TstError::InvalidConfig as i32);
    }

    #[test]
    fn null_recv_ts_returns_invalid_config() {
        let mut buf = [0u8; 188];
        let mut n: usize = 0;
        let rc = unsafe {
            tst_rtp_receiver_recv_ts(std::ptr::null_mut(), buf.as_mut_ptr(), buf.len(), &mut n)
        };
        assert_eq!(rc, TstError::InvalidConfig as i32);
    }

    #[test]
    fn null_get_stats_returns_invalid_config() {
        let mut stats = TstReceiverStats::default();
        let rc = unsafe { tst_rtp_receiver_get_stats(std::ptr::null_mut(), &mut stats) };
        assert_eq!(rc, TstError::InvalidConfig as i32);
    }

    #[test]
    fn null_reset_stats_returns_invalid_config() {
        let rc = unsafe { tst_rtp_receiver_reset_stats(std::ptr::null_mut()) };
        assert_eq!(rc, TstError::InvalidConfig as i32);
    }

    #[test]
    fn small_buf_returns_invalid_config() {
        let url = std::ffi::CString::new("rtp://127.0.0.1:0").unwrap();
        let handle = unsafe { tst_rtp_recv_open(url.as_ptr()) };
        if handle.is_null() {
            return; // skip if bind fails in CI
        }
        let mut buf = [0u8; 100]; // too small for 188-byte packet
        let mut n: usize = 0;
        let rc = unsafe { tst_rtp_receiver_recv_ts(handle, buf.as_mut_ptr(), buf.len(), &mut n) };
        assert_eq!(rc, TstError::InvalidConfig as i32);
        unsafe { tst_rtp_receiver_close(handle) };
    }
}
