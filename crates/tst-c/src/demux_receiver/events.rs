//! `tst_demux_receiver_recv_event` + `tst_demux_receiver_cancel` —
//! the event-marshalling step and its side-channel cancel.
//!
//! Receives one typed `TstEvent` per call by walking the demuxer pull
//! loop and converting Rust `DemuxEvent` items into C-shaped events
//! against the per-handle `EventArena` (design §4.5 borrowed-buffer
//! lifetime). `_cancel` lives alongside `_recv_event` (rather than with
//! the `_open` / `_close` lifecycle in `mod.rs`) because its sole
//! purpose is to unblock a thread parked in `_recv_event`, and the two
//! must remain adjacent in `tstrans.h` for the Task 5 byte-identical
//! header contract (cbindgen emits all parent-module items first, then
//! sub-modules in declaration order). Sibling-managed variant lives in
//! `managed.rs`.

use super::TstDemuxReceiver;
use crate::error::{TstError, record_eos, record_shell_error, set_last_error};
use crate::event::TstEvent;
use std::sync::atomic::Ordering;
use tst_pipeline::ShellErrorKind;

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
        Err(e)
            if e.kind == ShellErrorKind::TransportBroken
                && !was_cancelled.load(Ordering::Acquire) =>
        {
            // Same Broken-on-non-cancelled → EOS mapping as Phase 2's
            // tst_receiver_recv_packet (peer FIN surfaces as Broken
            // from libsrt; ManagedRecvTransport retries internally,
            // so a Broken reaching the plain receiver is a peer close).
            record_eos();
            TstError::EndOfStream as i32
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
pub unsafe extern "C" fn tst_demux_receiver_cancel(p: *mut TstDemuxReceiver) -> libc::c_int {
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
