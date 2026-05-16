//! `tst_receiver_t` (plain) and `tst_managed_receiver_t` (managed).
//!
//! One `_recv_packet` call = one 188-byte MPEG-TS packet. The underlying
//! `tst_pipeline::Receiver` runs the HUNT → VERIFY → LOCKED sync state
//! machine — bytes lost while scanning for the next aligned packet are
//! counted in `TstReceiverStats::{bytes_skipped_for_sync, resync_events}`
//! and are otherwise invisible to the caller.
//!
//! Cancellation contract: `_cancel` unblocks a thread parked in
//! `_recv_packet` within ~3-10 ms (one libsrt I/O cycle). The cancel
//! signal is delivered through a side-channel `Arc<dyn TransportCancel>`
//! field captured at `_open` time, not through the handle's `Mutex` —
//! so `_cancel` does not deadlock against a concurrent `_recv_packet`.

use crate::error::record_transport_error;
use crate::handle::Handle;
use crate::mux_sender::{parse_c_srt_url, parse_c_srt_url_listener};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use tst_pipeline::Receiver;
use tst_pipeline::TransportCancel;
use tst_srt::SrtTransport;
use tst_srt::SrtUrl;
use tst_srt::url::Mode;

// ------------------------------------------------------------------
// tst_receiver_t
// ------------------------------------------------------------------

pub struct TstReceiver {
    inner: Handle<Receiver<SrtTransport>>,
    /// Cancel handle snapshotted at `_open` time. Reaches the underlying
    /// libsrt socket so a blocked `_recv_packet` returns without waiting
    /// on the handle's `Mutex`.
    cancel: Option<Arc<dyn TransportCancel + Send + Sync>>,
    /// Set by `_cancel` and `_close` so the recv path can distinguish
    /// caller-initiated shutdown (returns `TST_E_CLOSED`) from peer FIN
    /// (returns `TST_E_END_OF_STREAM`).
    was_cancelled: Arc<AtomicBool>,
}

/// Open a `tst_receiver_t`. Accepts `srt://host:port?...` URLs;
/// URL with `?mode=listener` is routed through the listener path
/// (equivalent to calling `tst_receiver_open_listener`).
///
/// Returns `NULL` with `TST_E_INVALID_CONFIG` set in the thread-local
/// last-error for any malformed URL, unsupported key, unknown key, or
/// invalid value. `TST_E_TRANSPORT` set on connect/bind failure.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_receiver_open(
    srt_url: *const libc::c_char,
) -> *mut TstReceiver {
    crate::panic::ffi_catch(std::ptr::null_mut(), || {
        let url = match unsafe { parse_c_srt_url(srt_url) } {
            Ok(u) => u,
            Err(()) => return std::ptr::null_mut(),
        };
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
/// Empty-host URLs like `srt://:7000` are accepted directly; the parser's
/// requirement for an explicit `?mode=listener` does not apply here because
/// the entry-point name is already the authoritative listener signal.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_receiver_open_listener(
    srt_url: *const libc::c_char,
) -> *mut TstReceiver {
    crate::panic::ffi_catch(std::ptr::null_mut(), || {
        let url = match unsafe { parse_c_srt_url_listener(srt_url) } {
            Ok(u) => u,
            Err(()) => return std::ptr::null_mut(),
        };
        open_listener_inner(url)
    })
}

fn open_caller_inner(url: SrtUrl) -> *mut TstReceiver {
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

fn open_listener_inner(url: SrtUrl) -> *mut TstReceiver {
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

fn finish_open(transport: SrtTransport) -> *mut TstReceiver {
    let rx = Receiver::new(transport);
    let cancel = rx.cancel_handle();
    let was_cancelled = Arc::new(AtomicBool::new(false));
    Box::into_raw(Box::new(TstReceiver {
        inner: Handle::new(rx),
        cancel,
        was_cancelled,
    }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_receiver_close(p: *mut TstReceiver) {
    if p.is_null() {
        return;
    }
    let boxed = unsafe { Box::from_raw(p) };
    // Set the cancel flag and trip the libsrt-level cancel so any
    // concurrent recv_packet on this handle (multi-threaded misuse)
    // returns promptly with TST_E_CLOSED rather than TST_E_END_OF_STREAM.
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
            tst_receiver_close(std::ptr::null_mut());
        }
    }
}
