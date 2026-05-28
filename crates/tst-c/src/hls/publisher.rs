//! `TstPublisher` handle type + the universal `tst_publisher_*` data-path
//! entry points + the HLS-specific `tst_hls_publisher_*` accessors.
//!
//! A `Publisher` is **not a transport** — it is an outbound-only,
//! segment-aware sink (see `tst_core::publisher::Publisher`). Today the
//! only concrete publisher is the HLS publisher, which runs an internal
//! tokio HTTP server serving the rolling `.m3u8` playlist + `.ts`
//! segments. Dropping a live `TstPublisher` shuts that server down.
//!
//! The four universal symbols — `tst_publisher_push_ts`,
//! `tst_publisher_cut_segment`, `tst_publisher_finish`, and
//! `tst_publisher_get_stats` — mirror the `Publisher` trait method-for-
//! method and work against any future publisher kind. `tst_publisher_kind`
//! discriminates the concrete kind; the `tst_hls_publisher_*` accessors
//! reach the HLS-specific surface (richer stats, the bound socket address,
//! the rendered playlist).
//!
//! Unlike the `Handle<T>` wrapper used by transport handles, `TstPublisher`
//! stores `Option<PublisherImpl>` directly: `Publisher::finish` consumes
//! the inner publisher by value (`self`), which a `Mutex<Option<T>>` can
//! support but a `Handle` (built around `&mut T` / `&T` closures) cannot.
//! After `_finish` the inner is `None` (terminal) — subsequent push/cut
//! calls return `HlsFinished`.

use std::os::raw::c_char;

use tst_core::publisher::Publisher;

use crate::error::{TstError, hls_error_to_code, set_last_error};
use crate::stats::{TstHlsStats, TstPublisherStats};

// ---------------------------------------------------------------------------
// Handle type + concrete-publisher enum
// ---------------------------------------------------------------------------

/// Concrete publisher behind a `TstPublisher`. One variant per publisher
/// kind; today only HLS. Mux-publisher-driven and builder-driven paths
/// both produce this same enum.
pub(crate) enum PublisherImpl {
    /// HLS publisher (internal tokio HTTP server + on-disk segments).
    Hls(tst_tcp::hls::HlsPublisher),
}

/// Discriminator for the concrete publisher behind a `TstPublisher`,
/// returned by [`tst_publisher_kind`].
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TstPublisherKind {
    /// HLS publisher.
    Hls = 0,
}

/// Opaque handle for a segment-aware publisher sink.
///
/// Returned by [`crate::hls::builder::tst_hls_publisher_builder_build`] (and
/// by `tst_mux_publisher_finish_into_publisher`). Freed with
/// [`tst_publisher_free`].
///
/// `inner` is `None` only after [`tst_publisher_finish`] consumes the
/// publisher (terminal state); push/cut/stats on a finished handle return
/// `TST_E_HLS_FINISHED`.
pub struct TstPublisher {
    pub(crate) inner: Option<PublisherImpl>,
}

// ---------------------------------------------------------------------------
// Universal Publisher-trait-mirror entry points
// ---------------------------------------------------------------------------

/// Push pre-muxed MPEG-TS bytes for the current segment.
///
/// `bytes` MUST be a whole multiple of 188 (one or more MPEG-TS packets);
/// the HLS publisher rejects unaligned buffers with `TST_E_HLS_CONFIG`.
/// `(NULL, 0)` is accepted and is a no-op.
///
/// Returns 0 on success, a negative `TST_E_*` code on failure.
/// `TST_E_HLS_FINISHED` (-36) if the publisher has been finished.
///
/// # Safety
///
/// `p` must be a valid non-freed `*mut TstPublisher`. `bytes` must be
/// readable for `len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_publisher_push_ts(
    p: *mut TstPublisher,
    bytes: *const u8,
    len: usize,
) -> libc::c_int {
    crate::panic::ffi_catch(TstError::Internal as i32, || {
        let Some(handle) = (unsafe { p.as_mut() }) else {
            set_last_error(TstError::InvalidConfig, "null publisher pointer");
            return TstError::InvalidConfig as i32;
        };
        let slice = match unsafe { crate::ffi_slice::ffi_slice(bytes, len, "bytes") } {
            Ok(s) => s,
            Err(code) => return code,
        };
        match &mut handle.inner {
            Some(PublisherImpl::Hls(h)) => match h.push_ts(slice) {
                Ok(()) => 0,
                Err(e) => {
                    let code = hls_error_to_code(&e);
                    set_last_error(code, &format!("hls push_ts: {e}"));
                    code as i32
                }
            },
            None => {
                set_last_error(TstError::HlsFinished, "publisher already finished");
                TstError::HlsFinished as i32
            }
        }
    })
}

/// Hint that the next `tst_publisher_push_ts` should start a new segment.
///
/// Call this on keyframe boundaries so segments are decodable from byte
/// zero. May be a no-op for publishers that segment purely on duration.
///
/// Returns 0 on success, `TST_E_HLS_FINISHED` if finished, or another
/// negative `TST_E_*` code on failure.
///
/// # Safety
///
/// `p` must be a valid non-freed `*mut TstPublisher`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_publisher_cut_segment(p: *mut TstPublisher) -> libc::c_int {
    crate::panic::ffi_catch(TstError::Internal as i32, || {
        let Some(handle) = (unsafe { p.as_mut() }) else {
            set_last_error(TstError::InvalidConfig, "null publisher pointer");
            return TstError::InvalidConfig as i32;
        };
        match &mut handle.inner {
            Some(PublisherImpl::Hls(h)) => match h.cut_segment() {
                Ok(()) => 0,
                Err(e) => {
                    let code = hls_error_to_code(&e);
                    set_last_error(code, &format!("hls cut_segment: {e}"));
                    code as i32
                }
            },
            None => {
                set_last_error(TstError::HlsFinished, "publisher already finished");
                TstError::HlsFinished as i32
            }
        }
    })
}

/// Cleanly finalize the publisher: flush the pending segment, write the
/// terminating playlist tag (HLS `#EXT-X-ENDLIST`), and tear down the
/// internal HTTP server + file handles.
///
/// This consumes the inner publisher but leaves the handle allocated —
/// the caller must still `tst_publisher_free` it. After finish the handle
/// is terminal: subsequent push/cut/stats calls return
/// `TST_E_HLS_FINISHED`. Calling `_finish` twice returns
/// `TST_E_HLS_FINISHED` the second time.
///
/// Returns 0 on success, a negative `TST_E_*` code on failure.
///
/// # Safety
///
/// `p` must be a valid non-freed `*mut TstPublisher`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_publisher_finish(p: *mut TstPublisher) -> libc::c_int {
    crate::panic::ffi_catch(TstError::Internal as i32, || {
        let Some(handle) = (unsafe { p.as_mut() }) else {
            set_last_error(TstError::InvalidConfig, "null publisher pointer");
            return TstError::InvalidConfig as i32;
        };
        match handle.inner.take() {
            Some(PublisherImpl::Hls(h)) => match h.finish() {
                Ok(()) => 0,
                Err(e) => {
                    let code = hls_error_to_code(&e);
                    set_last_error(code, &format!("hls finish: {e}"));
                    code as i32
                }
            },
            None => {
                set_last_error(TstError::HlsFinished, "publisher already finished");
                TstError::HlsFinished as i32
            }
        }
    })
}

/// Snapshot the universal cross-publisher stats into `*out`.
///
/// Returns 0 on success, `TST_E_INVALID_CONFIG` if either pointer is null,
/// or `TST_E_HLS_FINISHED` if the publisher has been finished.
///
/// # Safety
///
/// `p` must be a valid `*mut TstPublisher`. `out` must point to a writable
/// `TstPublisherStats`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_publisher_get_stats(
    p: *mut TstPublisher,
    out: *mut TstPublisherStats,
) -> libc::c_int {
    crate::panic::ffi_catch(TstError::Internal as i32, || {
        let Some(handle) = (unsafe { p.as_ref() }) else {
            set_last_error(TstError::InvalidConfig, "null publisher pointer");
            return TstError::InvalidConfig as i32;
        };
        if out.is_null() {
            set_last_error(TstError::InvalidConfig, "null out pointer");
            return TstError::InvalidConfig as i32;
        }
        match &handle.inner {
            Some(PublisherImpl::Hls(h)) => {
                let stats = TstPublisherStats::from(&h.stats());
                unsafe { *out = stats };
                0
            }
            None => {
                set_last_error(TstError::HlsFinished, "publisher already finished");
                TstError::HlsFinished as i32
            }
        }
    })
}

/// Return the concrete publisher kind ([`TstPublisherKind`]) as a `u32`.
///
/// Returns `0` (`TstPublisherKind::Hls`) for an HLS publisher. On a null
/// or finished handle, still returns `0` and records nothing — the kind is
/// a static property of the handle's construction, not its liveness.
///
/// # Safety
///
/// `p` must be NULL or a valid non-freed `*mut TstPublisher`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_publisher_kind(p: *mut TstPublisher) -> u32 {
    crate::panic::ffi_catch(TstPublisherKind::Hls as u32, || {
        // Only one kind exists today; even a finished (inner == None)
        // handle was built as HLS. Future kinds will branch here.
        let _ = unsafe { p.as_ref() };
        TstPublisherKind::Hls as u32
    })
}

/// Close and free a `tst_publisher_t`.
///
/// If the publisher has not been finished, dropping it shuts down the
/// internal HTTP server (no `#EXT-X-ENDLIST` is written — call
/// `tst_publisher_finish` first for a clean VOD/event close).
///
/// Safe to call with `NULL` (no-op). After this call the pointer is
/// invalid; passing the same non-null pointer twice is undefined behavior
/// (use-after-free on the consumed `Box`).
///
/// # Safety
///
/// `p` must be NULL or a valid non-freed `*mut TstPublisher` returned by a
/// builder-build or mux-publisher-finish entry point.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_publisher_free(p: *mut TstPublisher) {
    crate::panic::ffi_catch((), || {
        if p.is_null() {
            return;
        }
        // Dropping the boxed TstPublisher drops Option<PublisherImpl>;
        // dropping a live HlsPublisher tears down its tokio HTTP server.
        drop(unsafe { Box::from_raw(p) });
    });
}

// ---------------------------------------------------------------------------
// HLS-specific accessors
// ---------------------------------------------------------------------------

/// Snapshot the HLS-specific richer stats into `*out`.
///
/// Returns 0 on success, `TST_E_INVALID_CONFIG` if either pointer is null,
/// or `TST_E_HLS_CONFIG` if the publisher is not an HLS publisher / has
/// been finished.
///
/// # Safety
///
/// `p` must be a valid `*mut TstPublisher`. `out` must point to a writable
/// `TstHlsStats`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_hls_publisher_get_hls_stats(
    p: *mut TstPublisher,
    out: *mut TstHlsStats,
) -> libc::c_int {
    crate::panic::ffi_catch(TstError::Internal as i32, || {
        let Some(handle) = (unsafe { p.as_ref() }) else {
            set_last_error(TstError::InvalidConfig, "null publisher pointer");
            return TstError::InvalidConfig as i32;
        };
        if out.is_null() {
            set_last_error(TstError::InvalidConfig, "null out pointer");
            return TstError::InvalidConfig as i32;
        }
        match &handle.inner {
            Some(PublisherImpl::Hls(h)) => {
                let stats = TstHlsStats::from(&h.hls_stats());
                unsafe { *out = stats };
                0
            }
            None => {
                set_last_error(
                    TstError::HlsConfig,
                    "publisher is finished or not an HLS publisher",
                );
                TstError::HlsConfig as i32
            }
        }
    })
}

/// Write the bound HTTP server socket address (`"ip:port"`) as a
/// NUL-terminated string into `buf` (capacity `buf_len`).
///
/// Returns the number of bytes written **excluding** the NUL terminator on
/// success, or a negative `TST_E_*` code on failure: `TST_E_INVALID_CONFIG`
/// if `buf` is null, `TST_E_HLS_CONFIG` if the address is unavailable
/// (server not bound / publisher finished / not an HLS publisher), or
/// `TST_E_HLS_CONFIG` with a "buffer too small" message if `buf_len` cannot
/// hold the address plus its NUL terminator.
///
/// Useful when the publisher was bound to an ephemeral port (`:0`) and the
/// caller needs the OS-assigned port to hand out the playlist URL.
///
/// # Safety
///
/// `p` must be a valid `*mut TstPublisher`. `buf` must be writable for
/// `buf_len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_hls_publisher_local_addr(
    p: *mut TstPublisher,
    buf: *mut c_char,
    buf_len: usize,
) -> libc::c_int {
    crate::panic::ffi_catch(TstError::Internal as i32, || {
        let Some(handle) = (unsafe { p.as_ref() }) else {
            set_last_error(TstError::InvalidConfig, "null publisher pointer");
            return TstError::InvalidConfig as i32;
        };
        if buf.is_null() {
            set_last_error(TstError::InvalidConfig, "null buf pointer");
            return TstError::InvalidConfig as i32;
        }
        let addr = match &handle.inner {
            Some(PublisherImpl::Hls(h)) => h.local_addr(),
            None => None,
        };
        match addr {
            Some(a) => unsafe { write_cstr_to_buf(&a.to_string(), buf, buf_len) },
            None => {
                set_last_error(
                    TstError::HlsConfig,
                    "local address unavailable (server not bound or publisher finished)",
                );
                TstError::HlsConfig as i32
            }
        }
    })
}

/// Render the current playlist as a NUL-terminated string into `buf`
/// (capacity `buf_len`).
///
/// `is_event` selects the terminal-tag flavor: pass `true` to render the
/// final playlist with `#EXT-X-ENDLIST` (as `finish` would write), `false`
/// for the live rolling playlist.
///
/// Returns the number of bytes written **excluding** the NUL terminator on
/// success, or a negative `TST_E_*` code: `TST_E_INVALID_CONFIG` if `buf`
/// is null, `TST_E_HLS_CONFIG` if the publisher is finished / not HLS, or
/// `TST_E_HLS_CONFIG` with a "buffer too small" message if `buf_len` cannot
/// hold the playlist plus its NUL terminator.
///
/// # Safety
///
/// `p` must be a valid `*mut TstPublisher`. `buf` must be writable for
/// `buf_len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_hls_publisher_render_playlist(
    p: *mut TstPublisher,
    is_event: bool,
    buf: *mut c_char,
    buf_len: usize,
) -> libc::c_int {
    crate::panic::ffi_catch(TstError::Internal as i32, || {
        let Some(handle) = (unsafe { p.as_ref() }) else {
            set_last_error(TstError::InvalidConfig, "null publisher pointer");
            return TstError::InvalidConfig as i32;
        };
        if buf.is_null() {
            set_last_error(TstError::InvalidConfig, "null buf pointer");
            return TstError::InvalidConfig as i32;
        }
        match &handle.inner {
            Some(PublisherImpl::Hls(h)) => {
                let playlist = h.render_playlist(is_event);
                unsafe { write_cstr_to_buf(&playlist, buf, buf_len) }
            }
            None => {
                set_last_error(
                    TstError::HlsConfig,
                    "publisher is finished or not an HLS publisher",
                );
                TstError::HlsConfig as i32
            }
        }
    })
}

/// Copy `s` plus a NUL terminator into the C buffer `buf` of capacity
/// `buf_len`. Returns the byte count written (excluding the NUL) on
/// success, or a negative `TST_E_HLS_CONFIG` code (with a recorded
/// last-error) if the buffer is too small. The caller has already null-
/// checked `buf`.
///
/// SAFETY: `buf` must be a valid writable pointer for `buf_len` bytes.
unsafe fn write_cstr_to_buf(s: &str, buf: *mut c_char, buf_len: usize) -> libc::c_int {
    let bytes = s.as_bytes();
    // Need room for the string plus the trailing NUL.
    if buf_len <= bytes.len() {
        set_last_error(
            TstError::HlsConfig,
            &format!(
                "buffer too small: need {} bytes (incl. NUL), have {buf_len}",
                bytes.len() + 1
            ),
        );
        return TstError::HlsConfig as i32;
    }
    // SAFETY: buf is valid for buf_len bytes (caller contract) and
    // bytes.len() + 1 <= buf_len from the guard above.
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf as *mut u8, bytes.len());
        *buf.add(bytes.len()) = 0;
    }
    bytes.len() as libc::c_int
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_free_is_safe() {
        unsafe { tst_publisher_free(std::ptr::null_mut()) };
    }

    #[test]
    fn null_push_ts_returns_invalid_config() {
        let rc = unsafe { tst_publisher_push_ts(std::ptr::null_mut(), std::ptr::null(), 0) };
        assert_eq!(rc, TstError::InvalidConfig as i32);
    }

    #[test]
    fn null_cut_segment_returns_invalid_config() {
        let rc = unsafe { tst_publisher_cut_segment(std::ptr::null_mut()) };
        assert_eq!(rc, TstError::InvalidConfig as i32);
    }

    #[test]
    fn null_finish_returns_invalid_config() {
        let rc = unsafe { tst_publisher_finish(std::ptr::null_mut()) };
        assert_eq!(rc, TstError::InvalidConfig as i32);
    }

    #[test]
    fn null_get_stats_returns_invalid_config() {
        let mut stats = TstPublisherStats::default();
        let rc = unsafe { tst_publisher_get_stats(std::ptr::null_mut(), &mut stats) };
        assert_eq!(rc, TstError::InvalidConfig as i32);
    }

    #[test]
    fn kind_is_hls() {
        assert_eq!(TstPublisherKind::Hls as u32, 0);
        // A null pointer still reports the (static) HLS kind.
        assert_eq!(unsafe { tst_publisher_kind(std::ptr::null_mut()) }, 0);
    }

    #[test]
    fn write_cstr_too_small_rejected() {
        let mut buf = [0i8; 3];
        let rc = unsafe { write_cstr_to_buf("hello", buf.as_mut_ptr(), buf.len()) };
        assert_eq!(rc, TstError::HlsConfig as i32);
    }

    #[test]
    fn write_cstr_exact_fit_writes_and_nul_terminates() {
        // "hi" needs 3 bytes (2 + NUL); a 3-byte buffer is exactly enough.
        let mut buf = [0i8; 3];
        let rc = unsafe { write_cstr_to_buf("hi", buf.as_mut_ptr(), buf.len()) };
        assert_eq!(rc, 2);
        assert_eq!(buf[0], b'h' as c_char);
        assert_eq!(buf[1], b'i' as c_char);
        assert_eq!(buf[2], 0);
    }
}
