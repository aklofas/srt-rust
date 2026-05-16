//! `tst_raw_receiver_t` (plain) and `tst_managed_raw_receiver_t` (managed).
//!
//! One `_recv` call = one inbound SRT message into the caller's buffer.
//! No MPEG-TS framing or sync recovery — that's `tst_ts_receiver_t`.
//!
//! Cancellation contract: `_cancel` unblocks a thread parked in `_recv`
//! within ~3-10 ms (one libsrt I/O cycle). The cancel signal is
//! delivered through a side-channel `Arc<dyn TransportCancel>` field
//! captured at `_open` time, not through the handle's `Mutex` — so
//! `_cancel` does not deadlock against a concurrent `_recv`.

use crate::error::record_transport_error;
use crate::handle::Handle;
use crate::mux_sender::parse_c_srt_url;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tst_pipeline::TransportCancel;
use tst_pipeline::RawReceiver;
use tst_srt::SrtTransport;
use tst_srt::SrtUrl;
use tst_srt::url::Mode;

// ------------------------------------------------------------------
// tst_raw_receiver_t
// ------------------------------------------------------------------

pub struct TstRawReceiver {
    inner: Handle<RawReceiver<SrtTransport>>,
    /// Cancel handle snapshotted at `_open` time. Reaches the underlying
    /// libsrt socket so a blocked `_recv` returns without waiting on
    /// the handle's `Mutex`.
    cancel: Option<Arc<dyn TransportCancel + Send + Sync>>,
    /// Set by `_cancel` and `_close` so the recv path can distinguish
    /// caller-initiated shutdown (returns `TST_E_CLOSED`) from peer FIN
    /// (returns `TST_E_END_OF_STREAM`).
    was_cancelled: Arc<AtomicBool>,
}

/// Open a `tst_raw_receiver_t`. Accepts `srt://host:port?...` URLs;
/// URL with `?mode=listener` is routed through the listener path
/// (equivalent to calling `tst_raw_receiver_open_listener`).
///
/// Returns `NULL` with `TST_E_INVALID_CONFIG` set in the thread-local
/// last-error for any malformed URL, unsupported key, unknown key, or
/// invalid value. `TST_E_TRANSPORT` set on connect/bind failure.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_raw_receiver_open(
    srt_url: *const libc::c_char,
) -> *mut TstRawReceiver {
    crate::panic::ffi_catch(std::ptr::null_mut(), || {
        let url = match unsafe { parse_c_srt_url(srt_url) } {
            Ok(u) => u,
            Err(()) => return std::ptr::null_mut(),
        };
        // URL-driven listener mode: route through listen path.
        if url.mode == Mode::Listener {
            return open_listener_inner(url);
        }
        open_caller_inner(url)
    })
}

/// Explicit listener-mode open. Forces listener mode regardless of any
/// `?mode=` URL value — the `_listener` suffix is authoritative. URLs
/// with `?mode=caller` are accepted and silently overridden.
///
/// (Phase 1 simplification of the design spec §4.2, which originally
/// proposed rejecting explicit `mode=caller` with `TST_E_INVALID_USAGE`.
/// The simpler rule is more forgiving and matches what most C consumers
/// expect from a `_listener`-suffixed entry point. The stricter check
/// can land in a future phase if a consumer asks.)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_raw_receiver_open_listener(
    srt_url: *const libc::c_char,
) -> *mut TstRawReceiver {
    crate::panic::ffi_catch(std::ptr::null_mut(), || {
        let url = match unsafe { parse_c_srt_url(srt_url) } {
            Ok(u) => u,
            Err(()) => return std::ptr::null_mut(),
        };
        open_listener_inner(url)
    })
}

fn open_caller_inner(url: SrtUrl) -> *mut TstRawReceiver {
    let mut socket_cfg = tst_srt::config::SocketConfig::default();
    url.overlay.apply_to_socket(&mut socket_cfg);
    let transport = match crate::connect::connect_srt(&url.host, url.port, &socket_cfg) {
        Ok(t) => t,
        Err(e) => {
            record_transport_error(&e);
            return std::ptr::null_mut();
        }
    };
    finish_open(transport)
}

fn open_listener_inner(url: SrtUrl) -> *mut TstRawReceiver {
    let mut listener_cfg = tst_srt::config::ListenerConfig::default();
    url.overlay.apply_to_listener(&mut listener_cfg);
    let transport = match crate::listen::listen_srt(&url.host, url.port, &listener_cfg) {
        Ok(t) => t,
        Err(e) => {
            record_transport_error(&e);
            return std::ptr::null_mut();
        }
    };
    finish_open(transport)
}

fn finish_open(transport: SrtTransport) -> *mut TstRawReceiver {
    let rx = RawReceiver::new(transport);
    let cancel = rx.cancel_handle();
    let was_cancelled = Arc::new(AtomicBool::new(false));
    Box::into_raw(Box::new(TstRawReceiver {
        inner: Handle::new(rx),
        cancel,
        was_cancelled,
    }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_raw_receiver_close(p: *mut TstRawReceiver) {
    if p.is_null() {
        return;
    }
    let boxed = unsafe { Box::from_raw(p) };
    // Set the cancel flag and trip the libsrt-level cancel so any
    // concurrent recv on this handle (multi-threaded misuse) returns
    // promptly with TST_E_CLOSED rather than TST_E_END_OF_STREAM.
    boxed.was_cancelled.store(true, std::sync::atomic::Ordering::Release);
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
            tst_raw_receiver_close(std::ptr::null_mut());
        }
    }
}
