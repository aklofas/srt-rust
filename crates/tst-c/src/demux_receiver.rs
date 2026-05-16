//! `tst_demux_receiver_t` (plain) and `tst_managed_demux_receiver_t`
//! (managed) — the typed-event receiver surface.
//!
//! One `_recv_event` call = one typed `TstEvent`. Events surface
//! ProgramMap topology, decoded Samples (NAL/OBU lists or raw audio
//! frames), KLV/private Metadata records, packet/PES discontinuities,
//! and non-conformance diagnostics. Pointer fields on the returned
//! event borrow from a per-handle `EventArena` until the next
//! `_recv_event` / `_close` call (design §4.5).
//!
//! Cancellation contract: `_cancel` unblocks a thread parked in
//! `_recv_event` within ~3-10 ms (one libsrt I/O cycle). Side-channel
//! `Arc<dyn TransportCancel>` + `Arc<AtomicBool>` pattern from
//! Phase 2's ts_receiver.rs — `_cancel` does not acquire the handle
//! Mutex so it does not deadlock against a concurrent `_recv_event`.

use crate::config::TstReconnectPolicy;
use crate::demux_config::TstDemuxConfig;
use crate::error::{TstError, record_eos, record_transport_error, set_last_error};
use crate::event::{EventArena, TstEvent};
use crate::handle::Handle;
use crate::mux_sender::{parse_c_srt_url, parse_c_srt_url_listener};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use tst_pipeline::DemuxReceiver;
use tst_pipeline::DemuxReceiverError;
use tst_pipeline::ManagedReceiveTransport;
use tst_pipeline::TransportCancel;
use tst_pipeline::TransportError;
use tst_srt::SrtTransport;
use tst_srt::SrtUrl;
use tst_srt::url::Mode;

// ------------------------------------------------------------------
// tst_demux_receiver_t
// ------------------------------------------------------------------

// fields arena, stream_stats_buf, cancel, was_cancelled consumed in Tasks 12-15
#[allow(dead_code)]
pub struct TstDemuxReceiver {
    inner: Handle<DemuxReceiver<SrtTransport>>,
    /// Reusable per-handle backing storage for `_recv_event` output.
    /// Wrapped in Mutex because event::convert() needs &mut access
    /// from inside the Handle's accessor closure (which takes the
    /// inner Mutex internally; we add a second one here because the
    /// arena's lifetime needs to outlive any single _recv_event
    /// borrow but be re-entrant only within this handle).
    arena: Mutex<EventArena>,
    /// Buffer for per-stream stats snapshot returned by
    /// `_get_stream_stats`. Repopulated on each call from the latest
    /// DemuxReceiver::stats().per_stream BTreeMap, capped at
    /// TST_STATS_MAX_STREAMS = 64. Borrowed-buffer lifetime per
    /// design §4.5 — valid until the next _get_stream_stats /
    /// _reset_stats / _close call.
    stream_stats_buf: Mutex<Vec<crate::stats::TstStreamStats>>,
    cancel: Option<Arc<dyn TransportCancel + Send + Sync>>,
    was_cancelled: Arc<AtomicBool>,
}

/// Open a `tst_demux_receiver_t` with default demux options.
/// Accepts `srt://host:port?...` URLs; `?mode=listener` routes through
/// the listener path (equivalent to `_open_listener`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_demux_receiver_open(
    srt_url: *const libc::c_char,
) -> *mut TstDemuxReceiver {
    crate::panic::ffi_catch(std::ptr::null_mut(), || {
        let url = match unsafe { parse_c_srt_url(srt_url) } {
            Ok(u) => u,
            Err(()) => return std::ptr::null_mut(),
        };
        if url.mode == Mode::Listener {
            return open_listener_inner(url, None);
        }
        open_caller_inner(url, None)
    })
}

/// Explicit listener-mode open with default demux options.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_demux_receiver_open_listener(
    srt_url: *const libc::c_char,
) -> *mut TstDemuxReceiver {
    crate::panic::ffi_catch(std::ptr::null_mut(), || {
        let url = match unsafe { parse_c_srt_url_listener(srt_url) } {
            Ok(u) => u,
            Err(()) => return std::ptr::null_mut(),
        };
        open_listener_inner(url, None)
    })
}

/// Open a `tst_demux_receiver_t` with a caller-supplied
/// `tst_demux_config_t`. The config is cloned-from at open time;
/// the caller still owns it and must `_free` it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_demux_receiver_open_with_config(
    srt_url: *const libc::c_char,
    cfg: *const TstDemuxConfig,
) -> *mut TstDemuxReceiver {
    crate::panic::ffi_catch(std::ptr::null_mut(), || {
        let url = match unsafe { parse_c_srt_url(srt_url) } {
            Ok(u) => u,
            Err(()) => return std::ptr::null_mut(),
        };
        let opts = unsafe { cfg.as_ref().map(|c| c.build_options()) };
        if url.mode == Mode::Listener {
            return open_listener_inner(url, opts);
        }
        open_caller_inner(url, opts)
    })
}

/// Explicit listener-mode open with a caller-supplied
/// `tst_demux_config_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_demux_receiver_open_listener_with_config(
    srt_url: *const libc::c_char,
    cfg: *const TstDemuxConfig,
) -> *mut TstDemuxReceiver {
    crate::panic::ffi_catch(std::ptr::null_mut(), || {
        let url = match unsafe { parse_c_srt_url_listener(srt_url) } {
            Ok(u) => u,
            Err(()) => return std::ptr::null_mut(),
        };
        let opts = unsafe { cfg.as_ref().map(|c| c.build_options()) };
        open_listener_inner(url, opts)
    })
}

fn open_caller_inner(
    url: SrtUrl,
    opts: Option<tst_core::mpegts::demux::DemuxerOptions>,
) -> *mut TstDemuxReceiver {
    let mut socket_cfg = tst_srt::config::SocketConfig::default();
    url.overlay.apply_to_socket(&mut socket_cfg);
    let transport = match crate::connect::connect_srt(&url.host, url.port, &socket_cfg) {
        Ok(t) => t,
        Err(e) => {
            record_transport_error(&e);
            return std::ptr::null_mut();
        }
    };
    finish_open(transport, opts)
}

fn open_listener_inner(
    url: SrtUrl,
    opts: Option<tst_core::mpegts::demux::DemuxerOptions>,
) -> *mut TstDemuxReceiver {
    let mut listener_cfg = tst_srt::config::ListenerConfig::default();
    url.overlay.apply_to_listener(&mut listener_cfg);
    let transport = match crate::listen::listen_srt(&url.host, url.port, &listener_cfg) {
        Ok(t) => t,
        Err(e) => {
            record_transport_error(&e);
            return std::ptr::null_mut();
        }
    };
    finish_open(transport, opts)
}

fn finish_open(
    transport: SrtTransport,
    opts: Option<tst_core::mpegts::demux::DemuxerOptions>,
) -> *mut TstDemuxReceiver {
    let rx = match opts {
        Some(o) => DemuxReceiver::with_demux_options(transport, o),
        None => DemuxReceiver::new(transport),
    };
    let cancel = rx.cancel_handle();
    let was_cancelled = Arc::new(AtomicBool::new(false));
    Box::into_raw(Box::new(TstDemuxReceiver {
        inner: Handle::new(rx),
        arena: Mutex::new(EventArena::new()),
        stream_stats_buf: Mutex::new(Vec::new()),
        cancel,
        was_cancelled,
    }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_demux_receiver_close(p: *mut TstDemuxReceiver) {
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
}

/// Block until one typed `TstEvent` is ready, then populate
/// `*out_event` with the converted event.
///
/// **Borrowed buffer lifetime (design §4.5):** pointer fields on
/// `*out_event` borrow from this handle's `EventArena`. They are
/// valid until the next `_recv_event` / `_close` call on the same
/// handle. Callers wanting longer lifetime memcpy out before the
/// next call.
///
/// Returns:
/// - `0` on success (`*out_event` populated; pointer fields borrow)
/// - `TST_E_END_OF_STREAM` (-12) on graceful peer close
/// - `TST_E_CLOSED` (-7) if the handle was `_cancel`'d or `_close`'d
/// - `TST_E_TRANSPORT` (-8) on transport failure
/// - `TST_E_INVALID_TS` (-3) on a demuxer error (strict-mode rejection
///   or unrecoverable packet malformation)
/// - `TST_E_INVALID_CONFIG` (-1) on null pointer arguments
///
/// On any non-zero return the contents of `*out_event` are unspecified.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_demux_receiver_recv_event(
    p: *mut TstDemuxReceiver,
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
            // SAFETY: out_event non-null per guard above. event::convert
            // writes through the pointer; pointer fields on the result
            // alias the arena Vecs (held under the arena Mutex for the
            // duration of this call; the arena Mutex is released before
            // the closure returns, but Vec base pointers are stable
            // until the next convert() call which re-clears them — see
            // the design §4.5 lifetime contract).
            unsafe {
                crate::event::convert(&mut arena, &ev, &mut *out_event);
            }
            0
        }
        Ok(None) => {
            if was_cancelled.load(Ordering::Acquire) {
                set_last_error(TstError::Closed, "receiver was cancelled or closed by caller");
                TstError::Closed as i32
            } else {
                record_eos();
                TstError::EndOfStream as i32
            }
        }
        Err(DemuxReceiverError::Transport(e)) => {
            // Same Broken-on-non-cancelled → EOS mapping as Phase 2's
            // tst_receiver_recv_packet (peer FIN surfaces as Broken
            // from libsrt; ManagedReceiveTransport retries internally,
            // so a Broken reaching the plain receiver is a peer close).
            if let TransportError::Broken(_) = &e {
                if !was_cancelled.load(Ordering::Acquire) {
                    record_eos();
                    return TstError::EndOfStream as i32;
                }
            }
            if let TransportError::Closed = &e {
                if was_cancelled.load(Ordering::Acquire) {
                    set_last_error(
                        TstError::Closed,
                        "receiver was cancelled or closed by caller",
                    );
                    return TstError::Closed as i32;
                }
                record_eos();
                return TstError::EndOfStream as i32;
            }
            record_transport_error(&e);
            unsafe { crate::error::tst_get_last_error() }
        }
        Err(DemuxReceiverError::Demux(e)) => {
            set_last_error(TstError::InvalidTs, &format!("demux error: {e}"));
            TstError::InvalidTs as i32
        }
        Err(e) => {
            set_last_error(TstError::Internal, &format!("unexpected demux receiver error: {e}"));
            TstError::Internal as i32
        }
    })
}

/// Cancel a `tst_demux_receiver_t`. Unblocks a thread parked in
/// `_recv_event` within one libsrt I/O cycle (~3-10 ms) by closing
/// the underlying libsrt socket. Safe to call from any thread.
/// Idempotent.
///
/// Returns 0 on success, `TST_E_INVALID_CONFIG` if the pointer is null.
///
/// After cancel, `_recv_event` returns `TST_E_CLOSED` (not
/// `TST_E_END_OF_STREAM`). The handle must still be `_close`'d to free.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_demux_receiver_cancel(
    p: *mut TstDemuxReceiver,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null receiver pointer");
        return TstError::InvalidConfig as i32;
    };
    handle.was_cancelled.store(true, Ordering::Release);
    if let Some(c) = &handle.cancel {
        c.cancel();
    }
    0
}

/// Snapshot stats for a `tst_demux_receiver_t` into `*out`.
///
/// Returns 0 on success, `TST_E_INVALID_CONFIG` if either pointer
/// is null, `TST_E_CLOSED` if the receiver has been closed.
///
/// NOTE: per-PID counters are NOT included on this struct — call
/// `tst_demux_receiver_get_stream_stats` to retrieve them.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_demux_receiver_get_stats(
    p: *mut TstDemuxReceiver,
    out: *mut crate::stats::TstDemuxReceiverStats,
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
        let stats = crate::stats::TstDemuxReceiverStats::from(&rx.stats());
        unsafe { *out = stats };
        0
    })
}

/// Reset stats counters for a `tst_demux_receiver_t` to zero.
/// Also invalidates the borrowed `_get_stream_stats` snapshot
/// (design §4.5).
///
/// Returns 0 on success, `TST_E_INVALID_CONFIG` if the pointer is
/// null, `TST_E_CLOSED` if the receiver has been closed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_demux_receiver_reset_stats(
    p: *mut TstDemuxReceiver,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null receiver pointer");
        return TstError::InvalidConfig as i32;
    };
    // Clear the stream_stats_buf so any borrowed snapshot becomes
    // a dangling pointer — caller contract documents this as the
    // invalidation moment.
    if let Ok(mut buf) = handle.stream_stats_buf.lock() {
        buf.clear();
    }
    handle.inner.with_inner_mut(|rx| {
        rx.reset_stats();
        0
    })
}

/// Snapshot per-PID stats for a `tst_demux_receiver_t` into the
/// handle's internal buffer; return a `(*const TstStreamStats, size_t)`
/// pair borrowing that buffer.
///
/// **Borrowed buffer lifetime (design §4.5):** `*out_array` is valid
/// until the next `_get_stream_stats` / `_reset_stats` / `_close`
/// call on the same handle. Callers wanting longer lifetime memcpy
/// the array out.
///
/// Capped at `TST_STATS_MAX_STREAMS = 64` entries (BTreeMap ordering
/// preserved by ascending PID); excess streams are silently dropped.
/// `program_number` field is `0` for now — populated once `StreamStats`
/// surfaces it (currently absent from `tst_core::mpegts::stats::StreamStats`).
///
/// Returns 0 on success, `TST_E_INVALID_CONFIG` on any null pointer
/// arg, or `TST_E_CLOSED` if the receiver has been closed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_demux_receiver_get_stream_stats(
    p: *mut TstDemuxReceiver,
    out_array: *mut *const crate::stats::TstStreamStats,
    out_count: *mut libc::size_t,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null receiver pointer");
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
        // The returned pointer borrows from buf, which lives on the
        // handle until the next _get_stream_stats / _reset_stats /
        // _close call (caller contract per design §4.5).
        unsafe {
            *out_array = buf.as_ptr();
            *out_count = buf.len();
        }
        0
    })
}

// ------------------------------------------------------------------
// tst_managed_demux_receiver_t
// ------------------------------------------------------------------

pub struct TstManagedDemuxReceiver {
    inner: Handle<DemuxReceiver<ManagedReceiveTransport<SrtTransport>>>,
    arena: Mutex<EventArena>,
    stream_stats_buf: Mutex<Vec<crate::stats::TstStreamStats>>,
    cancel: Option<Arc<dyn TransportCancel + Send + Sync>>,
    was_cancelled: Arc<AtomicBool>,
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
    opts: Option<tst_core::mpegts::demux::DemuxerOptions>,
) -> *mut TstManagedDemuxReceiver {
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
    let factory: Box<dyn FnMut() -> Result<SrtTransport, TransportError> + Send> =
        Box::new(move || crate::connect::connect_srt(&host, port, &cfg_for_reconnect));
    let managed = ManagedReceiveTransport::new(initial, factory, policy);
    finish_managed_open(managed, opts)
}

fn managed_open_listener_inner(
    url: SrtUrl,
    policy: tst_pipeline::ReconnectPolicy,
    opts: Option<tst_core::mpegts::demux::DemuxerOptions>,
) -> *mut TstManagedDemuxReceiver {
    let mut listener_cfg = tst_srt::config::ListenerConfig::default();
    url.overlay.apply_to_listener(&mut listener_cfg);
    let initial = match crate::listen::listen_srt(&url.host, url.port, &listener_cfg) {
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
        Box::new(move || crate::listen::listen_srt(&host, port, &cfg_for_relisten));
    let managed = ManagedReceiveTransport::new(initial, factory, policy);
    finish_managed_open(managed, opts)
}

fn finish_managed_open(
    managed: ManagedReceiveTransport<SrtTransport>,
    opts: Option<tst_core::mpegts::demux::DemuxerOptions>,
) -> *mut TstManagedDemuxReceiver {
    let rx = match opts {
        Some(o) => DemuxReceiver::with_demux_options(managed, o),
        None => DemuxReceiver::new(managed),
    };
    let cancel = rx.cancel_handle();
    let was_cancelled = Arc::new(AtomicBool::new(false));
    Box::into_raw(Box::new(TstManagedDemuxReceiver {
        inner: Handle::new(rx),
        arena: Mutex::new(EventArena::new()),
        stream_stats_buf: Mutex::new(Vec::new()),
        cancel,
        was_cancelled,
    }))
}

/// Block until one typed `TstEvent` is ready.
///
/// **Asymmetry with plain receiver:** plain `tst_demux_receiver_recv_event`
/// maps `TransportError::Broken` on a non-cancelled handle to
/// `TST_E_END_OF_STREAM`. The managed version does NOT apply that mapping —
/// `ManagedReceiveTransport` retries internally on Broken, so a Broken
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
        Err(DemuxReceiverError::Transport(e)) => {
            if let TransportError::Closed = &e {
                if was_cancelled.load(Ordering::Acquire) {
                    set_last_error(
                        TstError::Closed,
                        "receiver was cancelled or closed by caller",
                    );
                    return TstError::Closed as i32;
                }
                record_eos();
                return TstError::EndOfStream as i32;
            }
            record_transport_error(&e);
            unsafe { crate::error::tst_get_last_error() }
        }
        Err(DemuxReceiverError::Demux(e)) => {
            set_last_error(TstError::InvalidTs, &format!("demux error: {e}"));
            TstError::InvalidTs as i32
        }
        Err(e) => {
            set_last_error(
                TstError::Internal,
                &format!("unexpected demux receiver error: {e}"),
            );
            TstError::Internal as i32
        }
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
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null receiver pointer");
        return TstError::InvalidConfig as i32;
    };
    handle.was_cancelled.store(true, Ordering::Release);
    if let Some(c) = &handle.cancel {
        c.cancel();
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_managed_demux_receiver_close(p: *mut TstManagedDemuxReceiver) {
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_managed_demux_receiver_reset_stats(
    p: *mut TstManagedDemuxReceiver,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null receiver pointer");
        return TstError::InvalidConfig as i32;
    };
    if let Ok(mut buf) = handle.stream_stats_buf.lock() {
        buf.clear();
    }
    handle.inner.with_inner_mut(|rx| {
        rx.reset_stats();
        0
    })
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
        unsafe {
            *out_array = buf.as_ptr();
            *out_count = buf.len();
        }
        0
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::tst_get_last_error_str;

    #[test]
    fn null_close_is_safe() {
        unsafe {
            tst_demux_receiver_close(std::ptr::null_mut());
        }
    }

    #[test]
    fn null_handle_recv_event_returns_invalid_config() {
        let mut ev = TstEvent::default();
        let rc = unsafe { tst_demux_receiver_recv_event(std::ptr::null_mut(), &mut ev) };
        assert_eq!(rc, TstError::InvalidConfig as i32);
    }

    #[test]
    fn null_out_recv_event_returns_invalid_config() {
        let rc = unsafe {
            tst_demux_receiver_recv_event(std::ptr::null_mut(), std::ptr::null_mut())
        };
        assert_eq!(rc, TstError::InvalidConfig as i32);
    }

    #[test]
    fn null_cancel_returns_invalid_config() {
        let rc = unsafe { tst_demux_receiver_cancel(std::ptr::null_mut()) };
        assert_eq!(rc, TstError::InvalidConfig as i32);
    }

    #[test]
    fn null_get_stats_returns_invalid_config() {
        let mut stats = crate::stats::TstDemuxReceiverStats::default();
        let rc = unsafe { tst_demux_receiver_get_stats(std::ptr::null_mut(), &mut stats) };
        assert_eq!(rc, TstError::InvalidConfig as i32);
    }

    #[test]
    fn null_reset_stats_returns_invalid_config() {
        let rc = unsafe { tst_demux_receiver_reset_stats(std::ptr::null_mut()) };
        assert_eq!(rc, TstError::InvalidConfig as i32);
    }

    #[test]
    fn null_get_stream_stats_returns_invalid_config() {
        let mut arr: *const crate::stats::TstStreamStats = std::ptr::null();
        let mut count: libc::size_t = 0;
        let rc = unsafe {
            tst_demux_receiver_get_stream_stats(
                std::ptr::null_mut(),
                &mut arr,
                &mut count,
            )
        };
        assert_eq!(rc, TstError::InvalidConfig as i32);
    }

    #[test]
    fn managed_null_close_is_safe() {
        unsafe {
            tst_managed_demux_receiver_close(std::ptr::null_mut());
        }
    }

    #[test]
    fn managed_null_cancel_returns_invalid_config() {
        let rc = unsafe { tst_managed_demux_receiver_cancel(std::ptr::null_mut()) };
        assert_eq!(rc, TstError::InvalidConfig as i32);
    }

    #[test]
    fn managed_null_recv_event_returns_invalid_config() {
        let mut ev = TstEvent::default();
        let rc =
            unsafe { tst_managed_demux_receiver_recv_event(std::ptr::null_mut(), &mut ev) };
        assert_eq!(rc, TstError::InvalidConfig as i32);
    }

    #[test]
    fn managed_null_get_stats_returns_invalid_config() {
        let mut stats = crate::stats::TstDemuxReceiverStats::default();
        let rc = unsafe {
            tst_managed_demux_receiver_get_stats(std::ptr::null_mut(), &mut stats)
        };
        assert_eq!(rc, TstError::InvalidConfig as i32);
    }

    #[test]
    fn managed_null_reset_stats_returns_invalid_config() {
        let rc = unsafe { tst_managed_demux_receiver_reset_stats(std::ptr::null_mut()) };
        assert_eq!(rc, TstError::InvalidConfig as i32);
    }

    #[test]
    fn managed_null_get_stream_stats_returns_invalid_config() {
        let mut arr: *const crate::stats::TstStreamStats = std::ptr::null();
        let mut count: libc::size_t = 0;
        let rc = unsafe {
            tst_managed_demux_receiver_get_stream_stats(
                std::ptr::null_mut(),
                &mut arr,
                &mut count,
            )
        };
        assert_eq!(rc, TstError::InvalidConfig as i32);
    }

    /// Verify that an invalid SRT URL doesn't propagate a panic
    /// across the FFI boundary; instead returns null + sets
    /// last-error.
    #[test]
    fn open_invalid_url_returns_null_and_sets_last_error() {
        let bad = std::ffi::CString::new("not-a-url://").unwrap();
        let rx = unsafe { tst_demux_receiver_open(bad.as_ptr()) };
        assert!(rx.is_null());
        let last = unsafe { tst_get_last_error_str() };
        assert!(!last.is_null());
    }

    #[test]
    fn open_listener_invalid_url_returns_null() {
        let bad = std::ffi::CString::new("http://example.com").unwrap();
        let rx = unsafe { tst_demux_receiver_open_listener(bad.as_ptr()) };
        assert!(rx.is_null());
    }

    #[test]
    fn open_with_config_null_url_returns_null() {
        let cfg = unsafe { crate::demux_config::tst_demux_config_new() };
        let rx = unsafe {
            tst_demux_receiver_open_with_config(std::ptr::null(), cfg)
        };
        assert!(rx.is_null());
        unsafe { crate::demux_config::tst_demux_config_free(cfg) };
    }

    #[test]
    fn managed_open_with_config_null_url_returns_null() {
        let cfg = unsafe { crate::demux_config::tst_demux_config_new() };
        let rx = unsafe {
            tst_managed_demux_receiver_open_with_config(
                std::ptr::null(),
                std::ptr::null(),
                cfg,
            )
        };
        assert!(rx.is_null());
        unsafe { crate::demux_config::tst_demux_config_free(cfg) };
    }
}
