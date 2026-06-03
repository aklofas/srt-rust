//! C-string -> TcpUrl bridge.
//!
//! `tst_tcp::builder::TcpTransportBuilder::from_url` handles the full URL
//! grammar (`tcp://host:port` plain caller, `tcps://host:port` TLS caller,
//! `tcp://host:port?listen=1` listener, optional query params `?nodelay=`,
//! `?rcvbuf=`, `?sndbuf=`, `?pkt_size=`, `?connect_timeout=`).
//! This module is the C-string-to-Rust-str bridge plus error mapping.

use crate::error::{TstError, set_last_error};
use std::ffi::CStr;
use std::os::raw::c_char;

/// Parse a NUL-terminated C string `url_ptr` and return a borrowed `&str`
/// if it is valid UTF-8 and non-null.
///
/// On error, records `TstError::TcpConfig` to the thread-local last-error
/// and returns `None`.
///
/// SAFETY: `url_ptr` must be a NUL-terminated C string, valid for the
/// duration of the call.
pub(crate) unsafe fn parse_url_str<'a>(url_ptr: *const c_char) -> Option<&'a str> {
    if url_ptr.is_null() {
        set_last_error(TstError::TcpConfig, "url pointer is null");
        return None;
    }
    // SAFETY: caller-asserted NUL-termination + validity.
    let cstr = unsafe { CStr::from_ptr(url_ptr) };
    match cstr.to_str() {
        Ok(s) => Some(s),
        Err(_) => {
            set_last_error(TstError::TcpConfig, "url is not valid UTF-8");
            None
        }
    }
}
