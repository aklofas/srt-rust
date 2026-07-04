//! `TstRtpDemuxReceiver` handle type and data-path entry points.
//!
//! Open an RTP-backed `DemuxReceiver` with `tst_rtp_demux_receiver_open`.
//! Pull typed `TstEvent` items with `tst_rtp_demux_receiver_next_event`.
//! Cancel with `tst_rtp_demux_receiver_cancel`. Free with
//! `tst_rtp_demux_receiver_close`.
//!
//! Stats bodies (get_stats, get_socket_stats, get_stream_codec_stats,
//! reset_stats, get_stream_stats) are thin forwarders to generic impls
//! in `crate::transport_impls`. `next_event` and cancel stay family-local:
//! `next_event` needs `was_cancelled` discrimination between peer-EOF
//! and caller-cancel; cancel needs the `cancel` + `was_cancelled` Arc fields.

use std::os::raw::c_char;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use tst_core::RecvTransport;
use tst_pipeline::{DemuxReceiver, ShellErrorKind, TransportCancel};
use tst_rtp::{RtpRecvSocketBuilder, RtpRecvTransport};

use crate::demux_config::TstDemuxConfig;
use crate::error::{TstError, record_eos, record_shell_error, set_last_error};
use crate::event::{EventArena, TstEvent};
use crate::handle::Handle;

// ---------------------------------------------------------------------------
// Handle type
// ---------------------------------------------------------------------------

/// Opaque handle for an RTP-backed demux receiver.
///
/// Returned by [`tst_rtp_demux_receiver_open`]. Freed with
/// [`tst_rtp_demux_receiver_close`].
pub struct TstRtpDemuxReceiver {
    pub(crate) inner: Handle<DemuxReceiver<RtpRecvTransport>>,
    /// Reusable backing storage for `tst_rtp_demux_receiver_next_event` output.
    /// Allocated at open time so the data-path call never allocates on the hot path.
    /// Wrapped in Mutex for re-entrant safety within the Handle's closure.
    pub(crate) arena: Mutex<EventArena>,
    /// Per-stream stats snapshot buffer (borrowed-buffer design §4.5).
    pub(crate) stream_stats_buf: Mutex<Vec<crate::stats::TstStreamStats>>,
    pub(crate) cancel: Option<Arc<dyn TransportCancel + Send + Sync>>,
    /// Set by `_cancel` and `_close` so the recv path can distinguish
    /// caller-initiated shutdown (`TST_E_CLOSED`) from peer EOF
    /// (`TST_E_END_OF_STREAM`).
    pub(crate) was_cancelled: Arc<AtomicBool>,
}

// ---------------------------------------------------------------------------
// Open
// ---------------------------------------------------------------------------

/// Open an RTP-backed `DemuxReceiver`. `demux_cfg` may be `NULL`, in
/// which case default demux options apply (lenient / CFI-tolerant mode).
/// Returns `NULL` on error.
///
/// For unicast, pass `rtp://0.0.0.0:port`. For multicast, pass the group
/// address (`rtp://239.0.0.1:port?iface=eth0`).
///
/// # Safety
///
/// `url` is a NUL-terminated C string. `demux_cfg` may be NULL or a
/// valid `tst_demux_config_t*`. The returned handle must eventually be
/// freed with `tst_rtp_demux_receiver_close`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_rtp_demux_receiver_open(
    url: *const c_char,
    demux_cfg: *const TstDemuxConfig,
) -> *mut TstRtpDemuxReceiver {
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
        let receiver = if let Some(cfg) = unsafe { demux_cfg.as_ref() } {
            DemuxReceiver::with_demux_options(transport, cfg.build_options())
        } else {
            DemuxReceiver::new(transport)
        };
        Box::into_raw(Box::new(TstRtpDemuxReceiver {
            inner: Handle::new(receiver),
            arena: Mutex::new(EventArena::new()),
            stream_stats_buf: Mutex::new(Vec::new()),
            cancel,
            was_cancelled: Arc::new(AtomicBool::new(false)),
        }))
    })
}

// ---------------------------------------------------------------------------
// Close
// ---------------------------------------------------------------------------

/// Close and free a `tst_rtp_demux_receiver_t`.
///
/// Safe to call with `NULL` (no-op).
///
/// # Safety
///
/// `p` must be NULL or a valid non-freed `*mut TstRtpDemuxReceiver`
/// returned by `tst_rtp_demux_receiver_open`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_rtp_demux_receiver_close(p: *mut TstRtpDemuxReceiver) {
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

/// Block until one typed `TstEvent` is ready, then populate
/// `*out_event` with the converted event.
///
/// **Borrowed buffer lifetime (design §4.5):** pointer fields on
/// `*out_event` borrow from this handle's `EventArena`. They are
/// valid until the next `_next_event` / `_close` call on the same
/// handle. Callers wanting longer lifetime memcpy out before the
/// next call.
///
/// Returns:
/// - `0` on success (`*out_event` populated)
/// - `TST_E_END_OF_STREAM` (-12) on graceful peer close / EOF
/// - `TST_E_CLOSED` (-7) if the handle was `_cancel`'d or `_close`'d
/// - `TST_E_TRANSPORT` (-8) on transport failure
/// - `TST_E_INVALID_TS` (-3) on a demuxer error
/// - `TST_E_INVALID_CONFIG` (-1) on null pointer arguments
///
/// # Safety
///
/// `p` must be a valid non-freed `*mut TstRtpDemuxReceiver`. `out_event`
/// must be a valid writable `*mut TstEvent`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_rtp_demux_receiver_next_event(
    p: *mut TstRtpDemuxReceiver,
    out_event: *mut TstEvent,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null rtp demux receiver pointer");
        return TstError::InvalidConfig as i32;
    };
    if out_event.is_null() {
        set_last_error(TstError::InvalidConfig, "null out_event pointer");
        return TstError::InvalidConfig as i32;
    }
    let was_cancelled = handle.was_cancelled.clone();
    handle.inner.with_inner_mut(|rx| match rx.recv_event() {
        Ok(Some(ev)) => {
            let mut arena = handle.arena.lock().expect("event arena Mutex poisoned");
            // SAFETY: out_event non-null per guard above. event::convert
            // writes through the pointer; pointer fields on the result
            // alias arena Vecs (held under the arena Mutex for this call;
            // Vec base pointers are stable until the next convert() call —
            // see design §4.5 lifetime contract).
            unsafe { crate::event::convert(&mut arena, &ev, &mut *out_event) };
            0
        }
        Ok(None) => {
            if was_cancelled.load(Ordering::Acquire) {
                set_last_error(
                    TstError::Closed,
                    "rtp demux receiver was cancelled or closed by caller",
                );
                TstError::Closed as i32
            } else {
                record_eos();
                TstError::EndOfStream as i32
            }
        }
        Err(e)
            if e.kind == ShellErrorKind::TransportBroken
                && !was_cancelled.load(Ordering::Acquire) =>
        {
            // Broken on a non-cancelled handle means the peer closed — map to EOS.
            record_eos();
            TstError::EndOfStream as i32
        }
        Err(e) if e.kind == ShellErrorKind::EndOfStream || e.kind == ShellErrorKind::Closed => {
            if was_cancelled.load(Ordering::Acquire) {
                set_last_error(
                    TstError::Closed,
                    "rtp demux receiver was cancelled or closed by caller",
                );
                TstError::Closed as i32
            } else {
                record_eos();
                TstError::EndOfStream as i32
            }
        }
        Err(e) => record_shell_error(&e),
    })
}

/// Cancel a `tst_rtp_demux_receiver_t`. Signals the underlying RTP socket
/// to stop, unblocking any thread parked in `_next_event`. Safe to call
/// from any thread. Idempotent.
///
/// Returns 0 on success, `TST_E_INVALID_CONFIG` if the pointer is null.
///
/// After cancel, `_next_event` returns `TST_E_CLOSED`. The handle must
/// still be `_close`'d to free.
///
/// # Safety
///
/// `p` must be NULL or a valid non-freed `*mut TstRtpDemuxReceiver`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_rtp_demux_receiver_cancel(p: *mut TstRtpDemuxReceiver) -> libc::c_int {
    crate::panic::ffi_catch(TstError::Internal as i32, || {
        let Some(handle) = (unsafe { p.as_ref() }) else {
            set_last_error(TstError::InvalidConfig, "null rtp demux receiver pointer");
            return TstError::InvalidConfig as i32;
        };
        // Side-channel: do NOT acquire handle.inner's Mutex (a concurrent
        // next_event holds it). The was_cancelled flag + cancel-handle Arc
        // are accessible without locking.
        handle.was_cancelled.store(true, Ordering::Release);
        if let Some(c) = &handle.cancel {
            c.cancel();
        }
        0
    })
}

// ---------------------------------------------------------------------------
// Stats
// ---------------------------------------------------------------------------

/// Snapshot aggregate stats for a `tst_rtp_demux_receiver_t` into `*out`.
///
/// Returns 0 on success, `TST_E_INVALID_CONFIG` if either pointer is
/// null, or `TST_E_CLOSED` if the receiver has been closed.
///
/// NOTE: per-PID counters are NOT included here — call
/// `tst_rtp_demux_receiver_get_stream_stats` to retrieve them.
///
/// # Safety
///
/// `p` must be a valid `*mut TstRtpDemuxReceiver` opened via
/// `tst_rtp_demux_receiver_open`. `out` must point to a writable
/// `TstDemuxReceiverStats`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_rtp_demux_receiver_get_stats(
    p: *mut TstRtpDemuxReceiver,
    out: *mut crate::stats::TstDemuxReceiverStats,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null rtp demux receiver pointer");
        return TstError::InvalidConfig as i32;
    };
    unsafe { crate::transport_impls::demux_receiver_get_stats(&handle.inner, out) }
}

/// Read wire-level transport stats for the underlying RTP socket.
///
/// `out` MUST point to a writable `TstSocketStats`; the function zeros
/// the struct on failure.
///
/// Returns 0 on success, `TST_E_INVALID_CONFIG` if either pointer is null,
/// `TST_E_NOT_AVAILABLE` if no live stats are available, or
/// `TST_E_CLOSED` if the handle was closed.
///
/// # Safety
///
/// `p` must be a valid `*mut TstRtpDemuxReceiver` opened via
/// `tst_rtp_demux_receiver_open`. `out` must point to a writable
/// `TstSocketStats`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_rtp_demux_receiver_get_socket_stats(
    p: *mut TstRtpDemuxReceiver,
    out: *mut crate::stats::TstSocketStats,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null rtp demux receiver pointer");
        return TstError::InvalidConfig as i32;
    };
    unsafe {
        crate::transport_impls::demux_receiver_get_socket_stats(
            &handle.inner,
            out,
            "rtp demux receiver socket stats unavailable (transport not connected or closed)",
        )
    }
}

/// Snapshot codec-specific stats for one PID on a
/// `tst_rtp_demux_receiver_t`.
///
/// The returned struct is a tagged union — read `out->kind` first, then
/// the matching `out->u.<arm>` field.
///
/// # Errors
///
/// * `TST_E_INVALID_CONFIG` — `p` or `out` is null
/// * `TST_E_CLOSED` — handle was closed
/// * `TST_E_NOT_FOUND` — `pid` has never been observed on this handle
/// * `TST_E_INTERNAL` — internal panic caught at the FFI boundary
///
/// # Safety
///
/// `p` must be a valid pointer obtained from `tst_rtp_demux_receiver_open`.
/// `out` must be a writable `tst_stream_codec_stats_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_rtp_demux_receiver_get_stream_codec_stats(
    p: *mut TstRtpDemuxReceiver,
    pid: u16,
    out: *mut crate::stats::TstStreamCodecStats,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null rtp demux receiver pointer");
        return TstError::InvalidConfig as i32;
    };
    unsafe {
        crate::transport_impls::demux_receiver_get_stream_codec_stats(
            &handle.inner,
            pid,
            out,
            &format!(
                "codec stats not available for pid 0x{pid:04x} (pid has never been observed on this rtp demux receiver)"
            ),
        )
    }
}

/// Reset stats counters for a `tst_rtp_demux_receiver_t` to zero.
/// Also invalidates the borrowed `_get_stream_stats` snapshot
/// (design §4.5).
///
/// Returns 0 on success, `TST_E_INVALID_CONFIG` if the pointer is null,
/// or `TST_E_CLOSED` if the receiver has been closed.
///
/// # Safety
///
/// `p` must be a valid `*mut TstRtpDemuxReceiver` opened via
/// `tst_rtp_demux_receiver_open`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_rtp_demux_receiver_reset_stats(
    p: *mut TstRtpDemuxReceiver,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null rtp demux receiver pointer");
        return TstError::InvalidConfig as i32;
    };
    crate::transport_impls::demux_receiver_reset_stats(&handle.inner, &handle.stream_stats_buf)
}

/// Snapshot per-PID stats for a `tst_rtp_demux_receiver_t` into the
/// handle's internal buffer; return a `(*const TstStreamStats, size_t)`
/// pair borrowing that buffer.
///
/// **Borrowed buffer lifetime (design §4.5):** `*out_array` is valid
/// until the next `_get_stream_stats` / `_reset_stats` / `_close`
/// call on the same handle. Callers wanting longer lifetime memcpy
/// the array out.
///
/// Capped at `TST_STATS_MAX_STREAMS = 64` entries (ascending PID order).
///
/// Returns 0 on success, `TST_E_INVALID_CONFIG` on any null pointer
/// arg, or `TST_E_CLOSED` if the receiver has been closed.
///
/// # Safety
///
/// `p` must be a valid `*mut TstRtpDemuxReceiver` opened via
/// `tst_rtp_demux_receiver_open`. `out_array` and `out_count` must be
/// valid non-null pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_rtp_demux_receiver_get_stream_stats(
    p: *mut TstRtpDemuxReceiver,
    out_array: *mut *const crate::stats::TstStreamStats,
    out_count: *mut libc::size_t,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null rtp demux receiver pointer");
        return TstError::InvalidConfig as i32;
    };
    unsafe {
        crate::transport_impls::demux_receiver_get_stream_stats(
            &handle.inner,
            &handle.stream_stats_buf,
            out_array,
            out_count,
        )
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_close_is_safe() {
        unsafe { tst_rtp_demux_receiver_close(std::ptr::null_mut()) };
    }

    #[test]
    fn null_cancel_returns_invalid_config() {
        let rc = unsafe { tst_rtp_demux_receiver_cancel(std::ptr::null_mut()) };
        assert_eq!(rc, TstError::InvalidConfig as i32);
    }

    #[test]
    fn null_next_event_returns_invalid_config() {
        let mut ev = TstEvent::default();
        let rc = unsafe { tst_rtp_demux_receiver_next_event(std::ptr::null_mut(), &mut ev) };
        assert_eq!(rc, TstError::InvalidConfig as i32);
    }

    #[test]
    fn null_out_event_returns_invalid_config() {
        let rc = unsafe {
            tst_rtp_demux_receiver_next_event(std::ptr::null_mut(), std::ptr::null_mut())
        };
        assert_eq!(rc, TstError::InvalidConfig as i32);
    }

    #[test]
    fn null_get_stats_returns_invalid_config() {
        let mut stats = crate::stats::TstDemuxReceiverStats::default();
        let rc = unsafe { tst_rtp_demux_receiver_get_stats(std::ptr::null_mut(), &mut stats) };
        assert_eq!(rc, TstError::InvalidConfig as i32);
    }

    #[test]
    fn null_reset_stats_returns_invalid_config() {
        let rc = unsafe { tst_rtp_demux_receiver_reset_stats(std::ptr::null_mut()) };
        assert_eq!(rc, TstError::InvalidConfig as i32);
    }

    #[test]
    fn null_get_stream_stats_returns_invalid_config() {
        let mut arr: *const crate::stats::TstStreamStats = std::ptr::null();
        let mut count: libc::size_t = 0;
        let rc = unsafe {
            tst_rtp_demux_receiver_get_stream_stats(std::ptr::null_mut(), &mut arr, &mut count)
        };
        assert_eq!(rc, TstError::InvalidConfig as i32);
    }

    #[test]
    fn open_with_null_url_returns_null() {
        let p = unsafe { tst_rtp_demux_receiver_open(std::ptr::null(), std::ptr::null()) };
        assert!(p.is_null());
    }
}
