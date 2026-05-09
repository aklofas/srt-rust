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
        #[cfg(feature = "log")]
        install_log_handler();
    });
}

#[cfg(feature = "log")]
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
        // libsrt log levels (LOG_DEBUG=7, INFO=6, NOTICE=5, WARNING=4,
        // ERROR=3, CRITICAL=2, ALERT=1, EMERGENCY=0). Map roughly to log::Level.
        let level = match level {
            7 => log::Level::Trace,
            6 => log::Level::Debug,
            5 => log::Level::Info,
            4 => log::Level::Warn,
            _ => log::Level::Error,
        };
        if message.is_null() {
            return;
        }
        let msg = unsafe { CStr::from_ptr(message) }.to_string_lossy();
        log::log!(target: "srt", level, "{}", msg);
    }

    unsafe {
        srt_sys::srt_setloghandler(std::ptr::null_mut(), Some(forward));
    }
}
