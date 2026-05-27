//! librist global initialization + log-callback registration.
//!
//! librist exposes a logging-settings struct via `rist_logging_set` (an
//! alloc-or-update helper) and a global setter `rist_logging_set_global`.
//! We register a Rust shim once (idempotent via `OnceLock`) that forwards
//! every log line to the `tracing` crate at the appropriate level.

use std::ffi::CStr;
use std::os::raw::{c_char, c_int, c_void};
use std::sync::OnceLock;

use crate::error::RistError;

static INIT: OnceLock<()> = OnceLock::new();

/// Ensure librist's global state is initialized (idempotent).
/// Returns the librist version string on first call (cached forever).
pub(crate) fn ensure_init() -> Result<&'static str, RistError> {
    INIT.get_or_init(register_log_callback);
    Ok(librist_version())
}

/// Cached librist version string.
fn librist_version() -> &'static str {
    static V: OnceLock<String> = OnceLock::new();
    V.get_or_init(|| {
        let raw = unsafe { rist_sys::librist_version() };
        if raw.is_null() {
            return String::new();
        }
        unsafe { CStr::from_ptr(raw) }
            .to_str()
            .unwrap_or("")
            .to_owned()
    })
}

/// Cached librist API version string.
#[allow(dead_code)]
pub(crate) fn librist_api_version() -> &'static str {
    static V: OnceLock<String> = OnceLock::new();
    V.get_or_init(|| {
        let raw = unsafe { rist_sys::librist_api_version() };
        if raw.is_null() {
            return String::new();
        }
        unsafe { CStr::from_ptr(raw) }
            .to_str()
            .unwrap_or("")
            .to_owned()
    })
}

/// librist log-callback shim: maps librist log levels onto `tracing` macros.
unsafe extern "C" fn log_shim(
    _arg: *mut c_void,
    level: rist_sys::rist_log_level,
    msg: *const c_char,
) -> c_int {
    if msg.is_null() {
        return 0;
    }
    let s = match unsafe { CStr::from_ptr(msg) }.to_str() {
        Ok(s) => s.trim_end_matches('\n'),
        Err(_) => return 0,
    };
    if level <= rist_sys::rist_log_level_RIST_LOG_ERROR {
        tracing::error!(target: "librist", "{s}");
    } else if level == rist_sys::rist_log_level_RIST_LOG_WARN {
        tracing::warn!(target: "librist", "{s}");
    } else if level == rist_sys::rist_log_level_RIST_LOG_NOTICE
        || level == rist_sys::rist_log_level_RIST_LOG_INFO
    {
        tracing::info!(target: "librist", "{s}");
    } else {
        tracing::debug!(target: "librist", "{s}");
    }
    0
}

fn register_log_callback() {
    // rist_logging_set(out, level, cb, arg, address, logfp) — pass null
    // address/logfp; lib will allocate a new struct since the *out is NULL.
    let mut logging: *mut rist_sys::rist_logging_settings = std::ptr::null_mut();
    let rc = unsafe {
        rist_sys::rist_logging_set(
            &mut logging,
            rist_sys::rist_log_level_RIST_LOG_INFO,
            Some(log_shim),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if rc != 0 || logging.is_null() {
        tracing::warn!("rist_logging_set failed: rc={rc}");
        return;
    }
    // Register as the global logging settings so udpsocket_* layer (and
    // anything else inside librist that uses the global) routes through us.
    let rc = unsafe { rist_sys::rist_logging_set_global(logging) };
    if rc != 0 {
        tracing::warn!("rist_logging_set_global failed: rc={rc}");
    }
    // Park the pointer in a global OnceLock so future RistTransport /
    // RistRecvTransport constructors can pass it into rist_sender_create /
    // rist_receiver_create. librist's logging_settings is shareable.
    let _ = GLOBAL_LOGGING.set(LoggingPtr(logging));
}

/// Send/Sync newtype around `*mut rist_logging_settings` so we can park it
/// in a OnceLock. librist's logging_settings is intentionally shareable
/// across contexts.
pub(crate) struct LoggingPtr(pub(crate) *mut rist_sys::rist_logging_settings);
unsafe impl Send for LoggingPtr {}
unsafe impl Sync for LoggingPtr {}

pub(crate) static GLOBAL_LOGGING: OnceLock<LoggingPtr> = OnceLock::new();

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ensure_init_returns_version() {
        let v = ensure_init().unwrap();
        assert!(!v.is_empty(), "librist version should be non-empty");
    }

    #[test]
    fn ensure_init_is_idempotent() {
        let v1 = ensure_init().unwrap();
        let v2 = ensure_init().unwrap();
        assert_eq!(v1, v2);
    }

    #[test]
    fn api_version_is_nonempty() {
        let v = librist_api_version();
        assert!(!v.is_empty(), "librist API version should be non-empty");
    }
}
