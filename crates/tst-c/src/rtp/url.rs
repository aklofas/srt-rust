//! Helper: parse RTP URL string to [`tst_rtp::RtpUrl`] + report parse
//! errors into the thread-local last-error slot.
//!
//! C callers pass a URL like `rtp://239.1.2.3:5000?ttl=4&iface=eth0&pkt_size=1316`.
//! `tst_rtp::RtpUrl::parse` handles all of that — this module is the
//! C-string-to-Rust-str bridge plus error mapping.

use std::ffi::CStr;
use std::os::raw::c_char;

use tst_rtp::RtpUrl;

use crate::error::{TstError, set_last_error};

/// Parse a NUL-terminated C string as an `rtp://` URL.
///
/// On error, records the appropriate last-error code + message and
/// returns `None`. On success returns the parsed [`RtpUrl`].
///
/// # Safety
///
/// `url_ptr` must be a NUL-terminated C string that is valid (non-null,
/// readable) for the entire duration of this call.
pub(crate) unsafe fn parse_url(url_ptr: *const c_char) -> Option<RtpUrl> {
    if url_ptr.is_null() {
        set_last_error(TstError::InvalidConfig, "rtp url pointer is null");
        return None;
    }
    // SAFETY: caller asserts NUL-termination + validity.
    let cstr = unsafe { CStr::from_ptr(url_ptr) };
    let url_str = match cstr.to_str() {
        Ok(s) => s,
        Err(_) => {
            set_last_error(TstError::InvalidConfig, "rtp url is not valid UTF-8");
            return None;
        }
    };
    match RtpUrl::parse(url_str) {
        Ok(u) => Some(u),
        Err(e) => {
            set_last_error(
                TstError::InvalidConfig,
                &format!("rtp url parse error: {e}"),
            );
            None
        }
    }
}
