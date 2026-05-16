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

#[allow(unused_imports)]
use crate::config::TstReconnectPolicy;
use crate::demux_config::TstDemuxConfig;
#[allow(unused_imports)]
use crate::error::{TstError, record_eos, record_transport_error, set_last_error};
#[allow(unused_imports)]
use crate::event::{EventArena, TstEvent};
use crate::handle::Handle;
use crate::mux_sender::{parse_c_srt_url, parse_c_srt_url_listener};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use tst_pipeline::DemuxReceiver;
#[allow(unused_imports)]
use tst_pipeline::DemuxReceiverError;
#[allow(unused_imports)]
use tst_pipeline::ManagedReceiveTransport;
use tst_pipeline::TransportCancel;
#[allow(unused_imports)]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_close_is_safe() {
        unsafe {
            tst_demux_receiver_close(std::ptr::null_mut());
        }
    }
}
