//! C-string -> RistUrl bridge.
//!
//! `tst_rist::RistUrl::parse` handles the full URL grammar
//! (`rist://host:port` unicast send,
//! `rist://@host:port` receiver bind per the ffmpeg `@` convention,
//! `rist://group:port` multicast send, query params
//! `?profile=simple|main`, `?bandwidth=N`, `?buffer=N`,
//! `?aes-type=128|192|256&secret=...`, `?cname=...`).
//! This module is the C-string-to-Rust-str bridge plus error mapping.

use crate::error::{TstError, set_last_error};
use std::ffi::CStr;
use std::os::raw::c_char;

/// Parse a NUL-terminated C string `url_ptr` and return a borrowed `&str`
/// if it is valid UTF-8 and non-null.
///
/// On error, records `TstError::RistConfig` to the thread-local last-error
/// and returns `None`.
///
/// SAFETY: `url_ptr` must be a NUL-terminated C string, valid for the
/// duration of the call.
pub(crate) unsafe fn parse_url_str<'a>(url_ptr: *const c_char) -> Option<&'a str> {
    if url_ptr.is_null() {
        set_last_error(TstError::RistConfig, "url pointer is null");
        return None;
    }
    // SAFETY: caller-asserted NUL-termination + validity.
    let cstr = unsafe { CStr::from_ptr(url_ptr) };
    match cstr.to_str() {
        Ok(s) => Some(s),
        Err(_) => {
            set_last_error(TstError::RistConfig, "url is not valid UTF-8");
            None
        }
    }
}
