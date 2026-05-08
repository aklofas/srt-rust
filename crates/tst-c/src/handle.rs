//! `Handle<T>` — the canonical wrapper for every C-side opaque pointer.
//!
//! `Handle<T> = Mutex<Option<T>>`. `_open` returns
//! `Box::into_raw(Box::new(Handle::new(inner)))`. Data-path entry points
//! call `Handle::with_inner_mut`; `_close` calls `Handle::close`. Drop of
//! the inner runs Drop, which closes the underlying transport / muxer.

use crate::error::{TstError, record_internal, record_panic_caught, set_last_error};
use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Mutex;

/// Extract a best-effort detail string from a `catch_unwind` payload.
/// Handles the two common panic-payload types — `&'static str` (from
/// `panic!("foo")`) and `String` (from `panic!("{}", x)`); falls back
/// to a placeholder for anything else.
fn panic_payload_message(payload: &(dyn Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "non-string panic payload".to_string()
    }
}

#[allow(dead_code)]
pub(crate) struct Handle<T> {
    inner: Mutex<Option<T>>,
}

impl<T> Handle<T> {
    #[allow(dead_code)]
    pub(crate) fn new(value: T) -> Self {
        Self {
            inner: Mutex::new(Some(value)),
        }
    }

    /// Convert to a raw `*mut Self` pointer suitable for returning across
    /// the FFI boundary.
    #[allow(dead_code)]
    pub(crate) fn into_raw(self) -> *mut Self {
        Box::into_raw(Box::new(self))
    }

    /// Take ownership back from a raw pointer. Caller must guarantee the
    /// pointer was originally produced by `into_raw` and has not already
    /// been freed.
    #[allow(dead_code)]
    pub(crate) unsafe fn from_raw(ptr: *mut Self) -> Box<Self> {
        unsafe { Box::from_raw(ptr) }
    }

    /// Run `f` against `&mut T` if the handle is live. If the handle is
    /// closed, sets `TST_E_CLOSED` and returns its code.
    ///
    /// The closure is run inside `std::panic::catch_unwind`. A panic
    /// transitively reachable from any tst-c data-path call is caught
    /// at the FFI boundary, recorded as `TST_E_PANIC_CAUGHT`, and the
    /// inner state is dropped (subsequent calls on the same handle
    /// return `TST_E_CLOSED`). `AssertUnwindSafe` is sound here because
    /// we catch and clear; no further use of `T` happens after a panic.
    #[allow(dead_code)]
    pub(crate) fn with_inner_mut<F>(&self, f: F) -> i32
    where
        F: FnOnce(&mut T) -> i32,
    {
        let mut guard = match self.inner.lock() {
            Ok(g) => g,
            Err(_) => {
                record_internal("mutex poisoned");
                return TstError::Internal as i32;
            }
        };
        match guard.as_mut() {
            Some(t) => match catch_unwind(AssertUnwindSafe(|| f(t))) {
                Ok(rc) => rc,
                Err(payload) => {
                    let detail = panic_payload_message(&*payload);
                    record_panic_caught(&detail);
                    // After a panic the inner state is indeterminate.
                    // Drop it so subsequent calls return Closed rather
                    // than reusing potentially-corrupted state.
                    *guard = None;
                    TstError::PanicCaught as i32
                }
            },
            None => {
                set_last_error(TstError::Closed, "handle is closed");
                TstError::Closed as i32
            }
        }
    }

    /// Run `f` against `&T` if the handle is live (same close semantics).
    ///
    /// Mirrors the panic-isolation behavior of `with_inner_mut`: a
    /// panic in `f` is caught at the FFI boundary and the inner state
    /// is dropped. Even though `&T` did not mutate the inner directly,
    /// the panic could have left external state (global mutexes, file
    /// descriptors, etc.) in an indeterminate state — defense-in-depth
    /// drops the inner anyway.
    #[allow(dead_code)]
    pub(crate) fn with_inner_ref<F>(&self, f: F) -> i32
    where
        F: FnOnce(&T) -> i32,
    {
        let mut guard = match self.inner.lock() {
            Ok(g) => g,
            Err(_) => {
                record_internal("mutex poisoned");
                return TstError::Internal as i32;
            }
        };
        match guard.as_ref() {
            Some(t) => match catch_unwind(AssertUnwindSafe(|| f(t))) {
                Ok(rc) => rc,
                Err(payload) => {
                    let detail = panic_payload_message(&*payload);
                    record_panic_caught(&detail);
                    *guard = None;
                    TstError::PanicCaught as i32
                }
            },
            None => {
                set_last_error(TstError::Closed, "handle is closed");
                TstError::Closed as i32
            }
        }
    }

    /// Take the inner value (idempotent — second call is a no-op).
    /// Triggers Drop of the inner, which closes the underlying resource.
    #[allow(dead_code)]
    pub(crate) fn close(&self) {
        if let Ok(mut guard) = self.inner.lock() {
            *guard = None;
        }
    }
}

// ---------------------------------------------------------------------------
// Stream handle types (multi-stream `mpegts::mux` fan-out)
// ---------------------------------------------------------------------------

/// Opaque per-program ordinal for a video elementary stream. Obtained from
/// [`tst_mux_config_add_video_stream`] at config time and reused with the
/// `_video_to` push siblings on every muxer-owning C variant.
///
/// Handles are stable across the config→open boundary and across managed
/// reconnects. They encode `(program_index, within_program_index)` as a
/// packed `u32` (bits 4..=7 = program, bits 0..=3 = within). They are NOT
/// interchangeable between muxers.
pub type TstVideoStreamHandle = u32;

/// Opaque per-program ordinal for a KLV elementary stream. Same packed
/// encoding as [`TstVideoStreamHandle`].
pub type TstKlvStreamHandle = u32;

/// Sentinel returned by `tst_mux_config_add_*_stream` on failure.
/// On failure, the last-error is also populated; check
/// `tst_get_last_error()` for the negative `TST_E_*` code.
pub const TST_INVALID_STREAM_HANDLE: u32 = u32::MAX;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_inner_runs_when_live() {
        let h = Handle::new(7i32);
        let rc = h.with_inner_mut(|n| {
            *n += 1;
            0
        });
        assert_eq!(rc, 0);
        let final_value = h.with_inner_ref(|n| *n);
        assert_eq!(final_value, 8);
    }

    #[test]
    fn with_inner_after_close_returns_closed_code() {
        let h = Handle::new(7i32);
        h.close();
        let rc = h.with_inner_mut(|_| 0);
        assert_eq!(rc, TstError::Closed as i32);
    }

    #[test]
    fn close_is_idempotent() {
        let h = Handle::new(7i32);
        h.close();
        h.close();
    }

    #[test]
    fn panic_in_inner_closure_is_caught() {
        use crate::error::clear_last_error_for_test;
        clear_last_error_for_test();
        let h = Handle::new(7i32);
        let rc = h.with_inner_mut(|_| panic!("test panic"));
        assert_eq!(rc, TstError::PanicCaught as i32);
        // After a caught panic, the inner is dropped: subsequent calls
        // see a closed handle.
        let rc2 = h.with_inner_mut(|_| 0);
        assert_eq!(rc2, TstError::Closed as i32);
    }

    #[test]
    fn panic_in_inner_ref_closure_is_caught() {
        use crate::error::clear_last_error_for_test;
        clear_last_error_for_test();
        let h = Handle::new(7i32);
        let rc = h.with_inner_ref(|_| panic!("test panic ref"));
        assert_eq!(rc, TstError::PanicCaught as i32);
        // Defense-in-depth: even though &T didn't mutate, external state
        // could be in an indeterminate post-panic state, so we drop the
        // inner. Subsequent calls return Closed.
        let rc2 = h.with_inner_ref(|_| 0);
        assert_eq!(rc2, TstError::Closed as i32);
    }
}
