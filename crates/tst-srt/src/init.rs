//! Crate-private libsrt initialization.
//!
//! `srt_startup()` must be called once before any libsrt API use. Lazy via
//! `OnceLock` from any constructor that needs libsrt (`Socket::connect_with`,
//! `Listener::bind_with`, etc.).
//!
//! `srt_cleanup()` is **not** called automatically — see the design doc for
//! rationale (drop-order ambiguity vs. negligible OS-reclaimed leaks).

use std::sync::OnceLock;

static SRT_INITIALIZED: OnceLock<()> = OnceLock::new();

/// Idempotent libsrt initialization.
///
/// Panics if `srt_startup` returns < 0 — that's a process-fatal condition.
pub(crate) fn ensure_initialized() {
    SRT_INITIALIZED.get_or_init(|| {
        let rc = unsafe { srt_sys::srt_startup() };
        if rc < 0 {
            panic!("srt_startup() failed with rc={rc}; libsrt cannot be used");
        }
        install_log_handler();
    });
}

fn install_log_handler() {
    use std::ffi::{CStr, c_char, c_int, c_void};

    extern "C" fn forward(
        _opaque: *mut c_void,
        level: c_int,
        _file: *const c_char,
        _line: c_int,
        _area: *const c_char,
        message: *const c_char,
    ) {
        if message.is_null() {
            return;
        }
        let msg = unsafe { CStr::from_ptr(message) }.to_string_lossy();
        // libsrt log levels (LOG_DEBUG=7, INFO=6, NOTICE=5, WARNING=4,
        // ERROR=3, CRITICAL=2, ALERT=1, EMERGENCY=0). Map roughly to
        // tracing levels. `tracing::event!` requires a const-known level,
        // so dispatch via the per-level macros rather than a runtime value.
        match level {
            7 => tracing::trace!(target: "srt", "{}", msg),
            6 => tracing::debug!(target: "srt", "{}", msg),
            5 => tracing::info!(target: "srt", "{}", msg),
            4 => tracing::warn!(target: "srt", "{}", msg),
            _ => tracing::error!(target: "srt", "{}", msg),
        }
    }

    unsafe {
        srt_sys::srt_setloghandler(std::ptr::null_mut(), Some(forward));
    }
}
