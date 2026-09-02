//! `tst_managed_demux_receiver_t` — reconnect-aware sibling of the
//! plain `tst_demux_receiver_t`.
//!
//! Wraps a `ManagedDemuxReceiver<SrtTransport>`. Per-reconnect sync /
//! demux state reset is automatic — a `ReconnectDiscontinuity` event
//! surfaces from `_recv_event` after each transport reconnect
//! (validate-1 Sprint 4 F2 + followup-1). Open / lifecycle / event /
//! stats surfaces mirror the plain receiver one-for-one with `managed_`
//! infix on every C entry. The two notable shape changes vs the plain
//! sibling:
//!
//! * `_open*` family takes an optional `*const TstReconnectPolicy`.
//! * `_recv_event` does NOT apply the Broken→EOS mapping the plain
//!   receiver does (ManagedRecvTransport retries internally on Broken;
//!   a Broken reaching this function means reconnect attempts are
//!   exhausted — a hard transport failure, not end-of-stream).

use crate::config::TstReconnectPolicy;
use crate::demux_config::TstDemuxConfig;
use crate::error::{
    TstError, record_eos, record_shell_error, record_transport_error, set_last_error,
};
use crate::event::{EventArena, TstEvent};
use crate::handle::Handle;
use crate::sender::mux_sender::{parse_c_srt_url, parse_c_srt_url_listener};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use tst_pipeline::ManagedDemuxReceiver;
use tst_pipeline::ManagedDemuxReceiverConfig;
use tst_pipeline::ManagedRecvTransport;
use tst_pipeline::RecvEndReasonHandle;
use tst_pipeline::ShellErrorKind;
use tst_pipeline::TransportCancel;
use tst_pipeline::TransportError;
use tst_srt::SrtTransport;
use tst_srt::SrtUrl;
use tst_srt::url::Mode;

use crate::stream_end_reason::TstStreamEndReason;

pub struct TstManagedDemuxReceiver {
    inner: Handle<ManagedDemuxReceiver<SrtTransport>>,
    arena: Mutex<EventArena>,
    stream_stats_buf: Mutex<Vec<crate::stats::TstStreamStats>>,
    cancel: Option<Arc<dyn TransportCancel + Send + Sync>>,
    was_cancelled: Arc<AtomicBool>,
    /// End-reason handle snapshotted at open time, same
    /// capture-before-move timing as `cancel` — obtained from the
    /// `ManagedDemuxReceiver` BEFORE it is moved into `Handle::new(...)`
    /// in `finish_managed_open`. Stays readable after the receiver is
    /// closed, which is what lets `tst_managed_demux_receiver_end_reason`
    /// be polled from a watchdog thread side-channel, without acquiring
    /// `inner`'s Mutex. Read by `tst_managed_demux_receiver_end_reason`.
    end_reason: RecvEndReasonHandle,
}

/// Open a `tst_managed_demux_receiver_t` with default demux options.
/// URL-driven mode dispatch matches `tst_demux_receiver_open`.
/// `policy` is the reconnect policy; pass NULL for default.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_managed_demux_receiver_open(
    srt_url: *const libc::c_char,
    policy: *const TstReconnectPolicy,
) -> *mut TstManagedDemuxReceiver {
    crate::panic::ffi_catch(std::ptr::null_mut(), || {
        let policy = match unsafe { policy.as_ref() } {
            Some(p) => p.inner.clone(),
            None => tst_pipeline::ReconnectPolicy::default(),
        };
        let url = match unsafe { parse_c_srt_url(srt_url) } {
            Ok(u) => u,
            Err(()) => return std::ptr::null_mut(),
        };
        if url.mode == Mode::Listener {
            return managed_open_listener_inner(url, policy, None);
        }
        managed_open_caller_inner(url, policy, None)
    })
}

/// Explicit listener-mode open for the managed demux receiver.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_managed_demux_receiver_open_listener(
    srt_url: *const libc::c_char,
    policy: *const TstReconnectPolicy,
) -> *mut TstManagedDemuxReceiver {
    crate::panic::ffi_catch(std::ptr::null_mut(), || {
        let policy = match unsafe { policy.as_ref() } {
            Some(p) => p.inner.clone(),
            None => tst_pipeline::ReconnectPolicy::default(),
        };
        let url = match unsafe { parse_c_srt_url_listener(srt_url) } {
            Ok(u) => u,
            Err(()) => return std::ptr::null_mut(),
        };
        managed_open_listener_inner(url, policy, None)
    })
}

/// Open with a TstDemuxConfig override. URL-driven mode dispatch.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_managed_demux_receiver_open_with_config(
    srt_url: *const libc::c_char,
    policy: *const TstReconnectPolicy,
    cfg: *const TstDemuxConfig,
) -> *mut TstManagedDemuxReceiver {
    crate::panic::ffi_catch(std::ptr::null_mut(), || {
        let policy = match unsafe { policy.as_ref() } {
            Some(p) => p.inner.clone(),
            None => tst_pipeline::ReconnectPolicy::default(),
        };
        let url = match unsafe { parse_c_srt_url(srt_url) } {
            Ok(u) => u,
            Err(()) => return std::ptr::null_mut(),
        };
        let opts = unsafe { cfg.as_ref().map(|c| c.build_options()) };
        if url.mode == Mode::Listener {
            return managed_open_listener_inner(url, policy, opts);
        }
        managed_open_caller_inner(url, policy, opts)
    })
}

/// Explicit listener-mode open with a TstDemuxConfig override.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_managed_demux_receiver_open_listener_with_config(
    srt_url: *const libc::c_char,
    policy: *const TstReconnectPolicy,
    cfg: *const TstDemuxConfig,
) -> *mut TstManagedDemuxReceiver {
    crate::panic::ffi_catch(std::ptr::null_mut(), || {
        let policy = match unsafe { policy.as_ref() } {
            Some(p) => p.inner.clone(),
            None => tst_pipeline::ReconnectPolicy::default(),
        };
        let url = match unsafe { parse_c_srt_url_listener(srt_url) } {
            Ok(u) => u,
            Err(()) => return std::ptr::null_mut(),
        };
        let opts = unsafe { cfg.as_ref().map(|c| c.build_options()) };
        managed_open_listener_inner(url, policy, opts)
    })
}

fn managed_open_caller_inner(
    url: SrtUrl,
    policy: tst_pipeline::ReconnectPolicy,
    opts: Option<tst_core::mpegts::demux::DemuxerConfig>,
) -> *mut TstManagedDemuxReceiver {
    let mut socket_cfg = tst_srt::config::SocketConfig::default();
    url.overlay.apply_to_socket(&mut socket_cfg);
    let initial = match crate::sender::connect::connect_srt(&url.host, url.port, &socket_cfg) {
        Ok(t) => t,
        Err(e) => {
            record_transport_error(&e);
            return std::ptr::null_mut();
        }
    };
    let host = url.host.clone();
    let port = url.port;
    let cfg_for_reconnect = socket_cfg.clone();
    let factory: Box<dyn FnMut() -> Result<SrtTransport, TransportError> + Send> =
        Box::new(move || crate::sender::connect::connect_srt(&host, port, &cfg_for_reconnect));
    let managed = ManagedRecvTransport::new(initial, factory, policy);
    finish_managed_open(managed, opts)
}

fn managed_open_listener_inner(
    url: SrtUrl,
    policy: tst_pipeline::ReconnectPolicy,
    opts: Option<tst_core::mpegts::demux::DemuxerConfig>,
) -> *mut TstManagedDemuxReceiver {
    let mut listener_cfg = tst_srt::config::ListenerConfig::default();
    url.overlay.apply_to_listener(&mut listener_cfg);
    let initial = match crate::receiver::listen::listen_srt(&url.host, url.port, &listener_cfg) {
        Ok(t) => t,
        Err(e) => {
            record_transport_error(&e);
            return std::ptr::null_mut();
        }
    };
    let host = url.host.clone();
    let port = url.port;
    let cfg_for_relisten = listener_cfg.clone();
    let factory: Box<dyn FnMut() -> Result<SrtTransport, TransportError> + Send> =
        Box::new(move || crate::receiver::listen::listen_srt(&host, port, &cfg_for_relisten));
    let managed = ManagedRecvTransport::new(initial, factory, policy);
    finish_managed_open(managed, opts)
}

fn finish_managed_open(
    managed: ManagedRecvTransport<SrtTransport>,
    opts: Option<tst_core::mpegts::demux::DemuxerConfig>,
) -> *mut TstManagedDemuxReceiver {
    let rx = match opts {
        Some(o) => ManagedDemuxReceiver::with_demux_options(
            managed,
            o,
            ManagedDemuxReceiverConfig::default(),
        ),
        None => ManagedDemuxReceiver::new(managed, ManagedDemuxReceiverConfig::default()),
    };
    let cancel = rx.cancel_handle();
    let end_reason = rx.end_reason_handle();
    let was_cancelled = Arc::new(AtomicBool::new(false));
    Box::into_raw(Box::new(TstManagedDemuxReceiver {
        inner: Handle::new(rx),
        arena: Mutex::new(EventArena::new()),
        stream_stats_buf: Mutex::new(Vec::new()),
        cancel,
        was_cancelled,
        end_reason,
    }))
}

/// Block until one typed `TstEvent` is ready.
///
/// **Asymmetry with plain receiver:** plain `tst_demux_receiver_recv_event`
/// maps `TransportError::Broken` on a non-cancelled handle to
/// `TST_E_END_OF_STREAM`. The managed version does NOT apply that mapping —
/// `ManagedRecvTransport` retries internally on Broken, so a Broken
/// reaching this function means reconnect attempts are exhausted: a hard
/// transport failure (`TST_E_TRANSPORT`), not end-of-stream.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_managed_demux_receiver_recv_event(
    p: *mut TstManagedDemuxReceiver,
    out_event: *mut TstEvent,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null receiver pointer");
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
            unsafe {
                crate::event::convert(&mut arena, &ev, &mut *out_event);
            }
            0
        }
        Ok(None) => {
            if was_cancelled.load(Ordering::Acquire) {
                set_last_error(
                    TstError::Closed,
                    "receiver was cancelled or closed by caller",
                );
                TstError::Closed as i32
            } else {
                record_eos();
                TstError::EndOfStream as i32
            }
        }
        Err(e) if e.kind == ShellErrorKind::EndOfStream || e.kind == ShellErrorKind::Closed => {
            if was_cancelled.load(Ordering::Acquire) {
                set_last_error(
                    TstError::Closed,
                    "receiver was cancelled or closed by caller",
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

/// Cancel a `tst_managed_demux_receiver_t`. Same shape as the plain
/// sibling — side-channel cancel, no Mutex acquisition. Safe from
/// any thread. Idempotent.
///
/// Returns 0 on success, `TST_E_INVALID_CONFIG` if the pointer is null.
///
/// After cancel, `_recv_event` returns `TST_E_CLOSED` (not
/// `TST_E_END_OF_STREAM`). The handle must still be `_close`'d to free.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_managed_demux_receiver_cancel(
    p: *mut TstManagedDemuxReceiver,
) -> libc::c_int {
    crate::panic::ffi_catch(TstError::Internal as i32, || {
        let Some(handle) = (unsafe { p.as_ref() }) else {
            set_last_error(TstError::InvalidConfig, "null receiver pointer");
            return TstError::InvalidConfig as i32;
        };
        handle.was_cancelled.store(true, Ordering::Release);
        if let Some(c) = &handle.cancel {
            c.cancel();
        }
        0
    })
}

/// Read the recorded reason this `tst_managed_demux_receiver_t` receive
/// session ended, if any.
///
/// Writes `TstStreamEndReason::None` (returns `0`) when the session
/// hasn't ended yet, or ended through a path this arc doesn't
/// instrument — and in that case the thread-local last-error channel is
/// left untouched (any pending failure from an earlier call is still
/// readable). A recorded reason is data, not a getter failure — this
/// only returns a nonzero code for a null-pointer argument.
///
/// Reuses the RTP-side `TstStreamEndReason` enum — its RTSP-shaped
/// variant names (`SessionExpired`, `KeepaliveFailed`, `ProtocolError`)
/// are never produced on this SRT recv path; only three of its six
/// non-`None` variants apply here:
/// - `tst_pipeline::RecvEndReason::EndOfStream` → `CleanTeardown`
/// - `tst_pipeline::RecvEndReason::ReconnectExhausted` → `TransportFailed`
///   (the reconnect decorator exhausted its policy budget — the peer
///   never came back)
/// - `tst_pipeline::RecvEndReason::Cancelled` → `Cancelled`
///
/// **Last-error side effect on every ACTUALLY-recorded reason:** unlike
/// the "hasn't ended" case above, once the session has ended this getter
/// unconditionally resets the thread-local last-error channel to
/// `TST_E_SUCCESS` with an EMPTY detail message — none of the three
/// `RecvEndReason` variants above carry a detail string (unlike the RTP
/// side's `KeepaliveFailed`/`TransportFailed`/`ProtocolError`), so
/// `tst_get_last_error_str()` always reads `""` once a reason has been
/// recorded, never a stale message left over from some earlier,
/// unrelated failure. Same contract as
/// `tst_rtp_demux_receiver_end_reason` — see that function's doc for the
/// full rationale and `docs/binding-authors.md`. Read any pending
/// failure from an earlier call BEFORE calling this getter, or it is
/// overwritten.
///
/// Side-channel: reads directly off the end-reason handle captured at
/// open time WITHOUT acquiring this handle's data-path Mutex — same
/// rationale as `tst_managed_demux_receiver_cancel` (a concurrent
/// `_recv_event` may be blocked holding it). This is what makes the
/// getter safe to poll from a watchdog thread while another thread
/// drives `_recv_event`. One consequence: this call never itself
/// returns `TST_E_CLOSED` — after `_close` the whole handle is freed,
/// and calling anything on it, including this getter, is a
/// use-after-free the caller must avoid.
///
/// # Safety
///
/// `p` must be a valid non-freed `*mut TstManagedDemuxReceiver` opened
/// via one of the `tst_managed_demux_receiver_open*` functions. `out`
/// must point to a writable `TstStreamEndReason`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_managed_demux_receiver_end_reason(
    p: *mut TstManagedDemuxReceiver,
    out: *mut TstStreamEndReason,
) -> libc::c_int {
    crate::panic::ffi_catch(TstError::Internal as i32, || {
        let Some(handle) = (unsafe { p.as_ref() }) else {
            set_last_error(TstError::InvalidConfig, "null receiver pointer");
            return TstError::InvalidConfig as i32;
        };
        if out.is_null() {
            set_last_error(TstError::InvalidConfig, "null out pointer");
            return TstError::InvalidConfig as i32;
        }
        let reason = match handle.end_reason.get() {
            Some(r) => convert_recv_end_reason(&r),
            None => TstStreamEndReason::None,
        };
        // SAFETY: out non-null per guard above.
        unsafe { *out = reason };
        0
    })
}

/// Convert a recorded [`tst_pipeline::RecvEndReason`] to its C
/// discriminant. Only called by `tst_managed_demux_receiver_end_reason`
/// when it already holds a `Some` from `RecvEndReasonHandle::get()` —
/// i.e. every arm here corresponds to an ACTUALLY-RECORDED reason, never
/// the "hasn't ended yet" case (that short-circuits to
/// `TstStreamEndReason::None` at the call site without reaching this
/// function — see the getter's doc for why that split matters to the
/// last-error contract).
///
/// Every arm therefore unconditionally resets the thread-local
/// last-error channel to `TstError::Success` with an empty detail
/// message — `RecvEndReason` carries no per-variant message data (unlike
/// `tst_rtp::StreamEndReason`), so there is nothing to forward.
///
/// `RecvEndReason` is `#[non_exhaustive]` on the `tst-pipeline` side; a
/// future variant this binding doesn't know how to map yet degrades to
/// `None` rather than panicking, mirroring
/// `crate::rtp::end_reason::convert_end_reason`'s wildcard fallback.
fn convert_recv_end_reason(r: &tst_pipeline::RecvEndReason) -> TstStreamEndReason {
    use tst_pipeline::RecvEndReason;
    let converted = match r {
        RecvEndReason::EndOfStream => TstStreamEndReason::CleanTeardown,
        RecvEndReason::ReconnectExhausted => TstStreamEndReason::TransportFailed,
        RecvEndReason::Cancelled => TstStreamEndReason::Cancelled,
        _ => TstStreamEndReason::None,
    };
    set_last_error(TstError::Success, "");
    converted
}

/// Close and free a `tst_managed_demux_receiver_t`.
///
/// Safe to call with NULL (no-op). After this call the pointer is
/// invalid; passing the same non-null pointer twice is undefined
/// behavior (use-after-free on the consumed `Box`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_managed_demux_receiver_close(p: *mut TstManagedDemuxReceiver) {
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_managed_demux_receiver_get_stats(
    p: *mut TstManagedDemuxReceiver,
    out: *mut crate::stats::TstDemuxReceiverStats,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null receiver pointer");
        return TstError::InvalidConfig as i32;
    };
    unsafe { crate::transport_impls::managed_demux_receiver_get_stats(&handle.inner, out) }
}

/// Snapshot reconnect telemetry for a `tst_managed_demux_receiver_t` —
/// recv-side sibling of `tst_managed_sender_get_reconnect_stats` /
/// `tst_managed_mux_sender_get_reconnect_stats` /
/// `tst_managed_raw_sender_get_reconnect_stats`. Reuses the same
/// `tst_managed_transport_stats_t` struct.
///
/// Field semantics differ from the send-side getter in two ways:
///
/// * `gap_len`, `gap_messages_dropped`, `gap_bytes_dropped` are always
///   `0`. Unlike the send side's `ManagedTransport`, the recv-side
///   `ManagedRecvTransport`/`ManagedDemuxReceiver` has no gap buffer —
///   there is nothing to queue while disconnected on the receive path
///   (a receiver only ever consumes bytes that already arrived; it
///   cannot buffer bytes the peer hasn't sent yet), so eviction
///   telemetry is structurally inapplicable.
/// * `reconnect_attempts` equals `reconnect_successes`
///   (`ManagedDemuxReceiver::reconnects_count()`). The recv side tracks
///   no separate attempts counter distinct from successful rebuilds
///   (unlike the send side's `ManagedTransportStats::reconnect_attempts`,
///   which counts every `factory()` invocation including failed ones) —
///   this is a real asymmetry between the two sides, not a bug; it is
///   documented here rather than silently reported as `0`, which would
///   read as "never attempted" and be actively misleading while a
///   reconnect is in progress.
///
/// `reconnecting` and `reconnect_successes` come from
/// `ManagedDemuxReceiver::reconnecting()` / `reconnects_count()` and are
/// live — they reflect the current state through reconnects (read via
/// `with_inner_ref`, which works whether or not the inner transport is
/// currently present).
///
/// Returns 0 on success, `TST_E_INVALID_CONFIG` if either pointer is
/// null, or `TST_E_CLOSED` if the receiver has been closed.
///
/// # Safety
///
/// `p` must be a valid `*mut TstManagedDemuxReceiver` opened via one of
/// the `tst_managed_demux_receiver_open*` functions. `out` must point
/// to a writable `tst_managed_transport_stats_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_managed_demux_receiver_get_reconnect_stats(
    p: *mut TstManagedDemuxReceiver,
    out: *mut crate::stats::TstManagedTransportStats,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null receiver pointer");
        return TstError::InvalidConfig as i32;
    };
    if out.is_null() {
        set_last_error(TstError::InvalidConfig, "null out pointer");
        return TstError::InvalidConfig as i32;
    }
    handle.inner.with_inner_ref(|rx| {
        let successes = rx.reconnects_count();
        let stats = crate::stats::TstManagedTransportStats {
            reconnect_attempts: successes,
            reconnect_successes: successes,
            reconnecting: rx.reconnecting(),
            ..Default::default()
        };
        // SAFETY: out non-null per guard above.
        unsafe { *out = stats };
        0
    })
}

/// Managed sibling of [`tst_demux_receiver_get_stream_codec_stats`](super::stats::tst_demux_receiver_get_stream_codec_stats).
/// Returns the same values — codec stats live on the inner `Demuxer`,
/// so they persist across reconnect. No `TST_E_NOT_AVAILABLE` routing.
///
/// # Errors
///
/// * `TST_E_INVALID_CONFIG` — `p` or `out` is null
/// * `TST_E_CLOSED` — handle was closed via `tst_managed_demux_receiver_close`
/// * `TST_E_NOT_FOUND` — `pid` has never been observed on this handle
/// * `TST_E_INTERNAL` — internal panic caught at the FFI boundary
///
/// # Safety
///
/// `p` must be a valid pointer obtained from
/// `tst_managed_demux_receiver_open`; `out` must be a writable
/// `tst_stream_codec_stats_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_managed_demux_receiver_get_stream_codec_stats(
    p: *mut TstManagedDemuxReceiver,
    pid: u16,
    out: *mut crate::stats::TstStreamCodecStats,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null receiver pointer");
        return TstError::InvalidConfig as i32;
    };
    unsafe {
        crate::transport_impls::managed_demux_receiver_get_stream_codec_stats(
            &handle.inner,
            pid,
            out,
            &format!(
                "codec stats not available for pid 0x{pid:04x} (pid has never been observed on this demux receiver)"
            ),
        )
    }
}

/// Managed sibling of [`tst_demux_receiver_get_socket_stats`](super::stats::tst_demux_receiver_get_socket_stats). Returns
/// `TST_E_NOT_AVAILABLE` when the reconnect loop currently has no live
/// inner socket.
///
/// # Safety
///
/// Caller MUST ensure `p` is a valid `*mut TstManagedDemuxReceiver`
/// opened via `tst_managed_demux_receiver_open` and `out` points to a
/// writable `TstSocketStats`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_managed_demux_receiver_get_socket_stats(
    p: *mut TstManagedDemuxReceiver,
    out: *mut crate::stats::TstSocketStats,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null receiver pointer");
        return TstError::InvalidConfig as i32;
    };
    unsafe {
        crate::transport_impls::managed_demux_receiver_get_socket_stats(
            &handle.inner,
            out,
            "managed demux receiver socket stats unavailable (transport reconnecting or closed)",
        )
    }
}

/// Managed sibling of [`tst_demux_receiver_get_stream_last_seen_micros`](super::stats::tst_demux_receiver_get_stream_last_seen_micros).
/// Same semantics — `*out_epoch_micros` is `0` for a pid never observed on
/// this handle, 0 on success otherwise.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_managed_demux_receiver_get_stream_last_seen_micros(
    p: *mut TstManagedDemuxReceiver,
    pid: u16,
    out_epoch_micros: *mut u64,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null receiver pointer");
        return TstError::InvalidConfig as i32;
    };
    unsafe {
        crate::transport_impls::managed_demux_receiver_get_stream_last_seen_micros(
            &handle.inner,
            pid,
            out_epoch_micros,
        )
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_managed_demux_receiver_reset_stats(
    p: *mut TstManagedDemuxReceiver,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null receiver pointer");
        return TstError::InvalidConfig as i32;
    };
    crate::transport_impls::managed_demux_receiver_reset_stats(
        &handle.inner,
        &handle.stream_stats_buf,
    )
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_managed_demux_receiver_get_stream_stats(
    p: *mut TstManagedDemuxReceiver,
    out_array: *mut *const crate::stats::TstStreamStats,
    out_count: *mut libc::size_t,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null receiver pointer");
        return TstError::InvalidConfig as i32;
    };
    unsafe {
        crate::transport_impls::managed_demux_receiver_get_stream_stats(
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
//
// Null-pointer-only coverage lives here (no live socket required). Real
// end-reason / reconnect-stats assertions against a live managed demux
// receiver live in `bindings/c/tests/url_open/demux_receiver.rs`, which
// has the real-SRT-socket rendezvous harness these entry points need.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_end_reason_returns_invalid_config() {
        let mut out = TstStreamEndReason::None;
        let rc = unsafe { tst_managed_demux_receiver_end_reason(std::ptr::null_mut(), &mut out) };
        assert_eq!(rc, TstError::InvalidConfig as i32);
    }

    #[test]
    fn null_get_reconnect_stats_returns_invalid_config() {
        let mut out = crate::stats::TstManagedTransportStats::default();
        let rc = unsafe {
            tst_managed_demux_receiver_get_reconnect_stats(std::ptr::null_mut(), &mut out)
        };
        assert_eq!(rc, TstError::InvalidConfig as i32);
    }

    /// `RecvEndReason` variants carry no message data, so every
    /// ACTUALLY-recorded reason must reset last-error to `(Success, "")`
    /// — pinned directly against the conversion helper, independent of
    /// live-socket setup. Mirrors `convert_end_reason`'s own unit tests
    /// in `crate::rtp::end_reason`.
    #[test]
    fn convert_recv_end_reason_sets_empty_last_error_detail_for_every_variant() {
        for (reason, expected) in [
            (
                tst_pipeline::RecvEndReason::EndOfStream,
                TstStreamEndReason::CleanTeardown,
            ),
            (
                tst_pipeline::RecvEndReason::ReconnectExhausted,
                TstStreamEndReason::TransportFailed,
            ),
            (
                tst_pipeline::RecvEndReason::Cancelled,
                TstStreamEndReason::Cancelled,
            ),
        ] {
            crate::error::clear_last_error_for_test();
            let converted = convert_recv_end_reason(&reason);
            // TstStreamEndReason has no Debug impl; compare discriminants.
            assert_eq!(converted as i32, expected as i32, "mapping for {reason:?}");
            assert_eq!(
                crate::error::test_last_error_code(),
                TstError::Success as i32
            );
            assert_eq!(crate::error::test_last_error_msg(), "");
        }
    }
}
