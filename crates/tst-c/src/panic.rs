//! FFI panic isolation helper.
//!
//! Every outermost `extern "C"` body in `tst-c` is wrapped in
//! [`ffi_catch`], which converts a panic into a recorded
//! `TstError::PanicCaught` last-error plus a caller-supplied default
//! return value. This prevents Rust unwinding from crossing the C
//! frame, which is undefined behavior under `panic="unwind"` and an
//! abort (with no last-error visibility) under `panic="abort"`.
//!
//! The inner `Handle::with_inner_{mut,ref}` helpers in `handle.rs`
//! already wrap data-path closures; `ffi_catch` covers the open path
//! and config-builder setters where no `Handle` exists yet.
//!
//! # Default-value conventions
//!
//! Pass the right default per the entry point's return type:
//!
//! | Return type                  | Default to pass                          |
//! |------------------------------|------------------------------------------|
//! | `*mut T` (opaque handle)     | `std::ptr::null_mut()`                   |
//! | `*const T`                   | `std::ptr::null()` (or static fallback)  |
//! | `libc::c_int`                | `TstError::Internal as i32` (-10)        |
//! | `TstVideoStreamHandle` /     | `TST_INVALID_STREAM_HANDLE` (u32::MAX)   |
//! | `TstKlvStreamHandle`         |                                          |
//! | `TstProgramHandle`           | `TST_INVALID_PROGRAM_HANDLE`             |
//! | `()` (void, e.g., `_free`)   | `()`                                     |
//!
//! `TstError::Internal as i32` is the default for `c_int` returns
//! because `ffi_catch` already records `PanicCaught` to last-error
//! on the panic arm — the *return value* is just a signal that
//! something went wrong. The success path returns 0 or a specific
//! `TstError::* as i32` directly; the default is only observed on
//! actual panic, where `PanicCaught` is in last-error.

use crate::error::record_panic_caught;
use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind};

/// Run `f` inside `catch_unwind`. On panic, record `PanicCaught` to
/// the thread-local last-error (with a best-effort detail extracted
/// from the panic payload) and return `default`. On success, return
/// `f()`'s value unchanged.
///
/// `AssertUnwindSafe` is sound here because every caller in `tst-c`
/// satisfies one of:
///   - The closure body mutates only thread-local state (last-error)
///     and Box-allocated data the caller has not yet observed.
///   - The closure body mutates `&mut TstMuxConfig` (or sibling
///     builder), and a panic mid-mutation leaves the builder in a
///     consistent-but-partial state; subsequent calls either complete
///     the build or `tst_mux_config_free` it. Both outcomes are sound
///     because the inner `Vec` / `Option` fields don't have
///     cross-field invariants that a partial push would violate.
///   - The closure body returns a freshly-allocated `Box::into_raw(...)`
///     that the caller now owns; a panic before `Box::into_raw` simply
///     drops the in-progress `Box` (Rust unwinding semantics) and
///     leaks nothing.
pub(crate) fn ffi_catch<R, F>(default: R, f: F) -> R
where
    F: FnOnce() -> R,
{
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(value) => value,
        Err(payload) => {
            let detail = panic_payload_message(&*payload);
            record_panic_caught(&detail);
            default
        }
    }
}

/// Best-effort detail string from a `catch_unwind` payload. Mirrors
/// the helper in `handle.rs` to keep both panic-isolation paths
/// surfacing the same detail format.
fn panic_payload_message(payload: &(dyn Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "non-string panic payload".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{TstError, clear_last_error_for_test, tst_get_last_error};

    #[test]
    fn no_panic_returns_closure_value() {
        clear_last_error_for_test();
        let rc: i32 = ffi_catch(-10, || 42);
        assert_eq!(rc, 42);
        assert_eq!(unsafe { tst_get_last_error() }, 0);
    }

    #[test]
    fn panic_returns_default_and_records_last_error() {
        clear_last_error_for_test();
        let rc: i32 = ffi_catch(-10, || panic!("boom"));
        assert_eq!(rc, -10);
        assert_eq!(
            unsafe { tst_get_last_error() },
            TstError::PanicCaught as i32
        );
    }

    #[test]
    fn panic_with_formatted_string_payload_captures_detail() {
        clear_last_error_for_test();
        let _: i32 = ffi_catch(-10, || panic!("dynamic-{}", 42));
        let ptr = unsafe { crate::error::tst_get_last_error_str() };
        let cstr = unsafe { std::ffi::CStr::from_ptr(ptr) };
        let msg = cstr.to_str().unwrap();
        assert!(msg.contains("dynamic-42"), "got: {msg}");
    }

    #[test]
    fn null_ptr_default_for_pointer_returns() {
        clear_last_error_for_test();
        struct Dummy;
        let p: *mut Dummy = ffi_catch(std::ptr::null_mut(), || panic!("ptr panic"));
        assert!(p.is_null());
        assert_eq!(
            unsafe { tst_get_last_error() },
            TstError::PanicCaught as i32
        );
    }

    #[test]
    fn unit_default_for_void_returns() {
        clear_last_error_for_test();
        ffi_catch((), || panic!("void panic"));
        assert_eq!(
            unsafe { tst_get_last_error() },
            TstError::PanicCaught as i32
        );
    }
}
