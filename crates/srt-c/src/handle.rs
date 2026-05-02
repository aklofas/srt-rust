//! `Handle<T>` — the canonical wrapper for every C-side opaque pointer.
//!
//! `Handle<T> = Mutex<Option<T>>`. `_open` returns
//! `Box::into_raw(Box::new(Handle::new(inner)))`. Data-path entry points
//! call `Handle::with_inner_mut`; `_close` calls `Handle::close`. Drop of
//! the inner runs Drop, which closes the underlying transport / muxer.

use crate::error::{SrtcError, record_internal, set_last_error};
use std::sync::Mutex;

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
    /// closed, sets `SRTC_E_CLOSED` and returns its code.
    #[allow(dead_code)]
    pub(crate) fn with_inner_mut<F>(&self, f: F) -> i32
    where
        F: FnOnce(&mut T) -> i32,
    {
        let mut guard = match self.inner.lock() {
            Ok(g) => g,
            Err(_) => {
                record_internal("mutex poisoned");
                return SrtcError::Internal as i32;
            }
        };
        match guard.as_mut() {
            Some(t) => f(t),
            None => {
                set_last_error(SrtcError::Closed, "handle is closed");
                SrtcError::Closed as i32
            }
        }
    }

    /// Run `f` against `&T` if the handle is live (same close semantics).
    #[allow(dead_code)]
    pub(crate) fn with_inner_ref<F>(&self, f: F) -> i32
    where
        F: FnOnce(&T) -> i32,
    {
        let guard = match self.inner.lock() {
            Ok(g) => g,
            Err(_) => {
                record_internal("mutex poisoned");
                return SrtcError::Internal as i32;
            }
        };
        match guard.as_ref() {
            Some(t) => f(t),
            None => {
                set_last_error(SrtcError::Closed, "handle is closed");
                SrtcError::Closed as i32
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
        assert_eq!(rc, SrtcError::Closed as i32);
    }

    #[test]
    fn close_is_idempotent() {
        let h = Handle::new(7i32);
        h.close();
        h.close();
    }
}
