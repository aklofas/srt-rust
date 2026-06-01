//! C-string -> &str bridge for the HLS publisher entry points.
//!
//! The HLS surface takes free-form C strings in several places — the
//! `hls://` / `hlss://` builder URL, the bind address, the output
//! directory, basic-auth credentials, and TLS cert/key paths. This
//! module is the single C-string-to-Rust-str bridge plus error mapping;
//! it mirrors `crates/tst-c/src/udp/url.rs` exactly but records
//! `TstError::HlsConfig` on failure (the HLS family's config-error code).

use crate::error::{TstError, set_last_error};
use std::ffi::CStr;
use std::os::raw::c_char;

/// Parse a NUL-terminated C string `ptr` and return a borrowed `&str`
/// if it is valid UTF-8 and non-null.
///
/// On error, records `TstError::HlsConfig` to the thread-local last-error
/// and returns `None`. `name` is folded into the diagnostic so the caller
/// can tell which string argument was bad.
///
/// SAFETY: `ptr` must be a NUL-terminated C string, valid for the
/// duration of the call.
pub(crate) unsafe fn parse_str<'a>(ptr: *const c_char, name: &str) -> Option<&'a str> {
    if ptr.is_null() {
        set_last_error(TstError::HlsConfig, &format!("{name} pointer is null"));
        return None;
    }
    // SAFETY: caller-asserted NUL-termination + validity.
    let cstr = unsafe { CStr::from_ptr(ptr) };
    match cstr.to_str() {
        Ok(s) => Some(s),
        Err(_) => {
            set_last_error(TstError::HlsConfig, &format!("{name} is not valid UTF-8"));
            None
        }
    }
}
