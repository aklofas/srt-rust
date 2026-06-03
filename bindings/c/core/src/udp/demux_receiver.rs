//! `TstUdpDemuxReceiver` handle type and data-path entry points.
//!
//! Open a UDP-backed `DemuxReceiver` with `tst_udp_demux_receiver_open`.
//! Pull typed `TstEvent` items with `tst_udp_demux_receiver_next_event`.
//! Free with `tst_udp_demux_receiver_close`.
//!
//! Pattern mirrors `bindings/c/src/rtp/demux_receiver.rs` exactly —
//! `EventArena` borrowed-buffer lifetime (design §4.5), `ShellErrorKind`
//! → error-code mapping, and the per-PID stats borrowed buffer are all
//! identical.
//!
//! **No cancel:** the UDP transport does not expose a `cancel_handle()`,
//! so there is no `tst_udp_demux_receiver_cancel` entry point and no
//! cancel / `was_cancelled` side-channel. `_close` simply drops the
//! handle. To unblock a thread parked in `_next_event`, close the handle
//! from the same thread (or rely on the socket's receive-timeout
//! behavior). Without a caller-cancel path there is no
//! `TST_E_CLOSED`-vs-`TST_E_END_OF_STREAM` discrimination: a graceful
//! transport close maps to `TST_E_END_OF_STREAM`.

use std::os::raw::c_char;
use std::sync::Mutex;

use tst_pipeline::{DemuxReceiver, ShellErrorKind};
use tst_udp::{UdpRecvTransport, UdpRecvTransportBuilder};

use crate::demux_config::TstDemuxConfig;
use crate::error::{
    TstError, record_eos, record_not_available, record_not_found, record_shell_error,
    set_last_error,
};
use crate::event::{EventArena, TstEvent};
use crate::handle::Handle;

// ---------------------------------------------------------------------------
// Handle type
// ---------------------------------------------------------------------------

/// Opaque handle for a UDP-backed demux receiver.
///
/// Returned by [`tst_udp_demux_receiver_open`]. Freed with
/// [`tst_udp_demux_receiver_close`].
pub struct TstUdpDemuxReceiver {
    pub(crate) inner: Handle<DemuxReceiver<UdpRecvTransport>>,
    /// Reusable backing storage for `tst_udp_demux_receiver_next_event` output.
    /// Allocated at open time so the data-path call never allocates on the hot path.
    /// Wrapped in Mutex for re-entrant safety within the Handle's closure.
    pub(crate) arena: Mutex<EventArena>,
    /// Per-stream stats snapshot buffer (borrowed-buffer design §4.5).
    pub(crate) stream_stats_buf: Mutex<Vec<crate::stats::TstStreamStats>>,
}

// ---------------------------------------------------------------------------
// Open
// ---------------------------------------------------------------------------

/// Open a UDP-backed `DemuxReceiver`. `demux_cfg` may be `NULL`, in
/// which case default demux options apply (lenient / CFI-tolerant mode).
/// Returns `NULL` on error.
///
/// For unicast, pass `udp://0.0.0.0:port`. For multicast, pass the group
/// address with the ffmpeg `@` prefix (`udp://@239.0.0.1:port?iface=eth0`).
///
/// # Safety
///
/// `url` is a NUL-terminated C string. `demux_cfg` may be NULL or a
/// valid `tst_demux_config_t*`. The returned handle must eventually be
/// freed with `tst_udp_demux_receiver_close`.
#[cfg(feature = "udp")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_udp_demux_receiver_open(
    url: *const c_char,
    demux_cfg: *const TstDemuxConfig,
) -> *mut TstUdpDemuxReceiver {
    crate::panic::ffi_catch(std::ptr::null_mut(), || {
        let url_str = match unsafe { super::url::parse_url_str(url) } {
            Some(s) => s,
            None => return std::ptr::null_mut(),
        };
        let builder = match UdpRecvTransportBuilder::from_url(url_str) {
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
                set_last_error(code, &format!("udp recv build: {e}"));
                return std::ptr::null_mut();
            }
        };
        let receiver = if let Some(cfg) = unsafe { demux_cfg.as_ref() } {
            DemuxReceiver::with_demux_options(transport, cfg.build_options())
        } else {
            DemuxReceiver::new(transport)
        };
        Box::into_raw(Box::new(TstUdpDemuxReceiver {
            inner: Handle::new(receiver),
            arena: Mutex::new(EventArena::new()),
            stream_stats_buf: Mutex::new(Vec::new()),
        }))
    })
}

// ---------------------------------------------------------------------------
// Close
// ---------------------------------------------------------------------------

/// Close and free a `tst_udp_demux_receiver_t`.
///
/// Safe to call with `NULL` (no-op).
///
/// # Safety
///
/// `p` must be NULL or a valid non-freed `*mut TstUdpDemuxReceiver`
/// returned by `tst_udp_demux_receiver_open`.
#[cfg(feature = "udp")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_udp_demux_receiver_close(p: *mut TstUdpDemuxReceiver) {
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
/// - `TST_E_CLOSED` (-7) if the handle was `_close`'d
/// - `TST_E_TRANSPORT` (-8) on transport failure
/// - `TST_E_INVALID_TS` (-3) on a demuxer error
/// - `TST_E_INVALID_CONFIG` (-1) on null pointer arguments
///
/// # Safety
///
/// `p` must be a valid non-freed `*mut TstUdpDemuxReceiver`. `out_event`
/// must be a valid writable `*mut TstEvent`.
#[cfg(feature = "udp")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_udp_demux_receiver_next_event(
    p: *mut TstUdpDemuxReceiver,
    out_event: *mut TstEvent,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null udp demux receiver pointer");
        return TstError::InvalidConfig as i32;
    };
    if out_event.is_null() {
        set_last_error(TstError::InvalidConfig, "null out_event pointer");
        return TstError::InvalidConfig as i32;
    }
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
        // No caller-cancel side-channel on UDP — recv_event returning None
        // means the stream ended; map to EOS.
        Ok(None) => {
            record_eos();
            TstError::EndOfStream as i32
        }
        // A Closed / EndOfStream / peer-Broken condition all mean the stream
        // ended; map to EOS.
        Err(e)
            if e.kind == ShellErrorKind::TransportBroken
                || e.kind == ShellErrorKind::EndOfStream
                || e.kind == ShellErrorKind::Closed =>
        {
            record_eos();
            TstError::EndOfStream as i32
        }
        Err(e) => record_shell_error(&e),
    })
}

// ---------------------------------------------------------------------------
// Stats
// ---------------------------------------------------------------------------

/// Snapshot aggregate stats for a `tst_udp_demux_receiver_t` into `*out`.
///
/// Returns 0 on success, `TST_E_INVALID_CONFIG` if either pointer is
/// null, or `TST_E_CLOSED` if the receiver has been closed.
///
/// NOTE: per-PID counters are NOT included here — call
/// `tst_udp_demux_receiver_get_stream_stats` to retrieve them.
///
/// # Safety
///
/// `p` must be a valid `*mut TstUdpDemuxReceiver` opened via
/// `tst_udp_demux_receiver_open`. `out` must point to a writable
/// `TstDemuxReceiverStats`.
#[cfg(feature = "udp")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_udp_demux_receiver_get_stats(
    p: *mut TstUdpDemuxReceiver,
    out: *mut crate::stats::TstDemuxReceiverStats,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null udp demux receiver pointer");
        return TstError::InvalidConfig as i32;
    };
    if out.is_null() {
        set_last_error(TstError::InvalidConfig, "null out pointer");
        return TstError::InvalidConfig as i32;
    }
    handle.inner.with_inner_ref(|rx| {
        let stats = crate::stats::TstDemuxReceiverStats::from(&rx.stats());
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
/// `TST_E_NOT_AVAILABLE` if no live stats are available, or
/// `TST_E_CLOSED` if the handle was closed.
///
/// # Safety
///
/// `p` must be a valid `*mut TstUdpDemuxReceiver` opened via
/// `tst_udp_demux_receiver_open`. `out` must point to a writable
/// `TstSocketStats`.
#[cfg(feature = "udp")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_udp_demux_receiver_get_socket_stats(
    p: *mut TstUdpDemuxReceiver,
    out: *mut crate::stats::TstSocketStats,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null udp demux receiver pointer");
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
            "udp demux receiver socket stats unavailable (transport not connected or closed)",
        ),
    })
}

/// Snapshot codec-specific stats for one PID on a
/// `tst_udp_demux_receiver_t`.
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
/// `p` must be a valid pointer obtained from `tst_udp_demux_receiver_open`.
/// `out` must be a writable `tst_stream_codec_stats_t`.
#[cfg(feature = "udp")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_udp_demux_receiver_get_stream_codec_stats(
    p: *mut TstUdpDemuxReceiver,
    pid: u16,
    out: *mut crate::stats::TstStreamCodecStats,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null udp demux receiver pointer");
        return TstError::InvalidConfig as i32;
    };
    if out.is_null() {
        set_last_error(TstError::InvalidConfig, "null out pointer");
        return TstError::InvalidConfig as i32;
    }
    handle.inner.with_inner_ref(|rx| match rx.stream_codec_stats(pid) {
        Some(stats) => {
            unsafe { *out = crate::stats::codec_stats_to_c(stats) };
            0
        }
        None => record_not_found(&format!(
            "codec stats not available for pid 0x{pid:04x} (pid has never been observed on this udp demux receiver)"
        )),
    })
}

/// Reset stats counters for a `tst_udp_demux_receiver_t` to zero.
/// Also invalidates the borrowed `_get_stream_stats` snapshot
/// (design §4.5).
///
/// Returns 0 on success, `TST_E_INVALID_CONFIG` if the pointer is null,
/// or `TST_E_CLOSED` if the receiver has been closed.
///
/// # Safety
///
/// `p` must be a valid `*mut TstUdpDemuxReceiver` opened via
/// `tst_udp_demux_receiver_open`.
#[cfg(feature = "udp")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_udp_demux_receiver_reset_stats(
    p: *mut TstUdpDemuxReceiver,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null udp demux receiver pointer");
        return TstError::InvalidConfig as i32;
    };
    // Clear the stream_stats_buf so any borrowed snapshot is invalidated.
    if let Ok(mut buf) = handle.stream_stats_buf.lock() {
        buf.clear();
    }
    handle.inner.with_inner_mut(|rx| {
        rx.reset_stats();
        0
    })
}

/// Snapshot per-PID stats for a `tst_udp_demux_receiver_t` into the
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
/// `p` must be a valid `*mut TstUdpDemuxReceiver` opened via
/// `tst_udp_demux_receiver_open`. `out_array` and `out_count` must be
/// valid non-null pointers.
#[cfg(feature = "udp")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_udp_demux_receiver_get_stream_stats(
    p: *mut TstUdpDemuxReceiver,
    out_array: *mut *const crate::stats::TstStreamStats,
    out_count: *mut libc::size_t,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null udp demux receiver pointer");
        return TstError::InvalidConfig as i32;
    };
    if out_array.is_null() || out_count.is_null() {
        set_last_error(
            TstError::InvalidConfig,
            "null out_array or out_count pointer",
        );
        return TstError::InvalidConfig as i32;
    }
    handle.inner.with_inner_ref(|rx| {
        let stats = rx.stats();
        let mut buf = handle
            .stream_stats_buf
            .lock()
            .expect("stream_stats_buf Mutex poisoned");
        buf.clear();
        let cap = crate::stats::TST_STATS_MAX_STREAMS;
        for (pid, ss) in stats.per_stream.iter().take(cap) {
            let mut c_ss = crate::stats::TstStreamStats {
                pid: *pid,
                ..Default::default()
            };
            crate::stats::fill_stream_stats(&mut c_ss, ss);
            buf.push(c_ss);
        }
        // SAFETY: out_array / out_count non-null per guard above.
        // The returned pointer borrows from buf, which lives on the handle
        // until the next _get_stream_stats / _reset_stats / _close call.
        unsafe {
            *out_array = buf.as_ptr();
            *out_count = buf.len();
        }
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
        unsafe { tst_udp_demux_receiver_close(std::ptr::null_mut()) };
    }

    #[test]
    fn null_next_event_returns_invalid_config() {
        let mut ev = TstEvent::default();
        let rc = unsafe { tst_udp_demux_receiver_next_event(std::ptr::null_mut(), &mut ev) };
        assert_eq!(rc, TstError::InvalidConfig as i32);
    }

    #[test]
    fn null_out_event_returns_invalid_config() {
        let rc = unsafe {
            tst_udp_demux_receiver_next_event(std::ptr::null_mut(), std::ptr::null_mut())
        };
        assert_eq!(rc, TstError::InvalidConfig as i32);
    }

    #[test]
    fn null_get_stats_returns_invalid_config() {
        let mut stats = crate::stats::TstDemuxReceiverStats::default();
        let rc = unsafe { tst_udp_demux_receiver_get_stats(std::ptr::null_mut(), &mut stats) };
        assert_eq!(rc, TstError::InvalidConfig as i32);
    }

    #[test]
    fn null_reset_stats_returns_invalid_config() {
        let rc = unsafe { tst_udp_demux_receiver_reset_stats(std::ptr::null_mut()) };
        assert_eq!(rc, TstError::InvalidConfig as i32);
    }

    #[test]
    fn null_get_stream_stats_returns_invalid_config() {
        let mut arr: *const crate::stats::TstStreamStats = std::ptr::null();
        let mut count: libc::size_t = 0;
        let rc = unsafe {
            tst_udp_demux_receiver_get_stream_stats(std::ptr::null_mut(), &mut arr, &mut count)
        };
        assert_eq!(rc, TstError::InvalidConfig as i32);
    }

    #[test]
    fn open_with_null_url_returns_null() {
        let p = unsafe { tst_udp_demux_receiver_open(std::ptr::null(), std::ptr::null()) };
        assert!(p.is_null());
    }
}
