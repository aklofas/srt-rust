//! Shared C-string -> `&str` bridge for transport URL / config-string parsing.
//!
//! Every transport family that takes a raw C string for a URL or a bare
//! config value (bind address, output path, basic-auth user/pass, TLS
//! cert/key paths, ...) needs the same three checks: reject null, validate
//! UTF-8, and record a family-specific `TstError` code + a name for the
//! diagnostic message. This module holds the one shared implementation
//! (udp/tcp/rist/hls all delegate here). `rtp/url.rs` stays separate — it
//! additionally runs the string through `RtpUrl::parse` rather than just
//! handing back the borrowed `&str`.

use crate::error::{TstError, set_last_error};
use std::ffi::CStr;
use std::os::raw::c_char;

/// Parse a NUL-terminated C string `ptr` and return a borrowed `&str` if
/// it is valid UTF-8 and non-null.
///
/// On error, records `code` to the thread-local last-error with a message
/// naming `name` (e.g. `"url"`, `"bind_addr"`, `"cert_path"`) and returns
/// `None`.
///
/// # Safety
///
/// `ptr` must be a NUL-terminated C string, valid for the duration of the
/// call.
pub(crate) unsafe fn parse_c_str<'a>(
    ptr: *const c_char,
    code: TstError,
    name: &str,
) -> Option<&'a str> {
    if ptr.is_null() {
        set_last_error(code, &format!("{name} pointer is null"));
        return None;
    }
    // SAFETY: caller-asserted NUL-termination + validity.
    let cstr = unsafe { CStr::from_ptr(ptr) };
    match cstr.to_str() {
        Ok(s) => Some(s),
        Err(_) => {
            set_last_error(code, &format!("{name} is not valid UTF-8"));
            None
        }
    }
}
