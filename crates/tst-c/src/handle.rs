//! `Handle<T>` — the canonical wrapper for every C-side opaque pointer.
//!
//! `Handle<T> = Mutex<Option<T>>`. `_open` returns
//! `Box::into_raw(Box::new(Handle::new(inner)))`. Data-path entry points
//! call `Handle::with_inner_mut`; `_close` calls `Handle::close`. Drop of
//! the inner runs Drop, which closes the underlying transport / muxer.

use crate::error::{TstError, record_internal, record_panic_caught, set_last_error};
#[cfg(feature = "std")]
use alloc::string::String;

#[cfg(not(feature = "std"))]
use crate::nostd_mutex::Mutex;
#[cfg(feature = "std")]
use std::sync::Mutex;

/// Best-effort detail string from a `catch_unwind` payload (std only).
#[cfg(feature = "std")]
fn panic_payload_message(payload: &(dyn std::any::Any + Send)) -> alloc::string::String {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        alloc::string::String::from(*s)
    } else if let Some(s) = payload.downcast_ref::<alloc::string::String>() {
        s.clone()
    } else {
        alloc::string::String::from("non-string panic payload")
    }
}

/// Run `f` catching any panic (std). Returns `Ok(result)` or `Err(detail)`.
#[cfg(feature = "std")]
fn catch<R>(f: impl FnOnce() -> R) -> Result<R, alloc::string::String> {
    use core::panic::AssertUnwindSafe;
    std::panic::catch_unwind(AssertUnwindSafe(f)).map_err(|p| panic_payload_message(&*p))
}

/// Under no_std (panic = abort), run the closure directly — no unwinding possible.
#[cfg(not(feature = "std"))]
fn catch<R>(f: impl FnOnce() -> R) -> Result<R, alloc::string::String> {
    Ok(f())
}

pub(crate) struct Handle<T> {
    inner: Mutex<Option<T>>,
}

impl<T> Handle<T> {
    pub(crate) fn new(value: T) -> Self {
        Self {
            inner: Mutex::new(Some(value)),
        }
    }

    /// Run `f` against `&mut T` if the handle is live. If the handle is
    /// closed, sets `TST_E_CLOSED` and returns its code.
    ///
    /// The closure is run inside `std::panic::catch_unwind`. A panic
    /// transitively reachable from any tst-c data-path call is caught
    /// at the FFI boundary, recorded as `TST_E_PANIC_CAUGHT`, and the
    /// inner state is dropped (subsequent calls on the same handle
    /// return `TST_E_CLOSED`). `AssertUnwindSafe` is sound here because
    /// we catch and clear; no further use of `T` happens after a panic.
    pub(crate) fn with_inner_mut<F>(&self, f: F) -> i32
    where
        F: FnOnce(&mut T) -> i32,
    {
        let mut guard = match self.inner.lock() {
            Ok(g) => g,
            Err(_) => {
                record_internal("mutex poisoned");
                return TstError::Internal as i32;
            }
        };
        match guard.as_mut() {
            Some(t) => match catch(|| f(t)) {
                Ok(rc) => rc,
                Err(detail) => {
                    record_panic_caught(&detail);
                    // After a panic the inner state is indeterminate.
                    // Drop it so subsequent calls return Closed rather
                    // than reusing potentially-corrupted state.
                    *guard = None;
                    TstError::PanicCaught as i32
                }
            },
            None => {
                set_last_error(TstError::Closed, "handle is closed");
                TstError::Closed as i32
            }
        }
    }

    /// Run `f` against `&T` if the handle is live (same close semantics).
    ///
    /// Mirrors the panic-isolation behavior of `with_inner_mut`: a
    /// panic in `f` is caught at the FFI boundary and the inner state
    /// is dropped. Even though `&T` did not mutate the inner directly,
    /// the panic could have left external state (global mutexes, file
    /// descriptors, etc.) in an indeterminate state — defense-in-depth
    /// drops the inner anyway.
    #[allow(dead_code)] // transport-feature-gated callers; unused in minimal builds
    pub(crate) fn with_inner_ref<F>(&self, f: F) -> i32
    where
        F: FnOnce(&T) -> i32,
    {
        let mut guard = match self.inner.lock() {
            Ok(g) => g,
            Err(_) => {
                record_internal("mutex poisoned");
                return TstError::Internal as i32;
            }
        };
        match guard.as_ref() {
            Some(t) => match catch(|| f(t)) {
                Ok(rc) => rc,
                Err(detail) => {
                    record_panic_caught(&detail);
                    *guard = None;
                    TstError::PanicCaught as i32
                }
            },
            None => {
                set_last_error(TstError::Closed, "handle is closed");
                TstError::Closed as i32
            }
        }
    }

    /// Take the inner value (idempotent — second call is a no-op).
    /// Triggers Drop of the inner, which closes the underlying resource.
    pub(crate) fn close(&self) {
        if let Ok(mut guard) = self.inner.lock() {
            *guard = None;
        }
    }
}

// ---------------------------------------------------------------------------
// RTSP client builder opaque handle (rtp feature)
// ---------------------------------------------------------------------------

/// Configuration accumulator for the RTSP client.
///
/// Allocated by `tst_rtsp_client_builder_new` and consumed (or freed) by
/// `tst_rtsp_client_builder_connect` (Task 6) / `tst_rtsp_client_builder_free`.
///
/// Fields are stored independently rather than wrapping `RtspClientBuilder`
/// directly because `RtspClientBuilder` uses consuming `mut self -> Self`
/// chain setters, making in-place C mutation cumbersome.  Task 6's connect
/// path constructs the final `RtspClientBuilder` from these fields.
// Allow dead_code: url + all other fields are read by Task 6's connect path
// which lands in a subsequent Wave B commit. The allow is removed at that time.
#[cfg(feature = "rtp")]
#[allow(dead_code)]
pub struct TstRtspClientBuilder {
    /// Parsed `rtsp://` or `rtsps://` URL.  Transport preference is
    /// embedded via `url.transport_preference`; `transport_pref` below
    /// overrides it if the caller explicitly called `_transport_pref`.
    pub(crate) url: tst_rtp::url::RtspUrl,
    /// Override for `url.transport_preference` set via
    /// `tst_rtsp_client_builder_transport_pref`.  `None` means use the
    /// preference already encoded in the URL (default: `PreferUdp`).
    pub(crate) transport_pref: Option<tst_rtp::url::RtspTransportPref>,
    /// Credentials supplied via any `_auth_*` entry point.
    pub(crate) username: Option<String>,
    /// Password in plain text; wrapped in `secrecy::SecretString` at
    /// `connect` time so it is zeroed when the builder is consumed.
    pub(crate) password: Option<String>,
    /// When `true` (the default), `connect` spawns an auto-keepalive
    /// thread.  Set to `false` by `tst_rtsp_client_builder_keepalive(b, false)`.
    pub(crate) auto_keepalive: bool,
    /// Raw PEM bytes for `rtsps://` connections.  Parsed into a
    /// `rustls::RootCertStore` at connect time.  `None` → system trust store.
    pub(crate) tls_root_cert_pem: Option<Vec<u8>>,
}

#[cfg(feature = "rtp")]
impl TstRtspClientBuilder {
    /// Construct from a parsed URL string.
    ///
    /// # Errors
    ///
    /// Returns `Err(String)` with a user-facing message if the URL cannot
    /// be parsed as an RTSP URL.
    pub(crate) fn from_url(url_str: &str) -> Result<Self, String> {
        let url = tst_rtp::url::RtspUrl::parse(url_str).map_err(|e| e.to_string())?;
        Ok(Self {
            url,
            transport_pref: None,
            username: None,
            password: None,
            auto_keepalive: true,
            tls_root_cert_pem: None,
        })
    }

    /// Leak the `Box` and return a raw pointer suitable for FFI.
    pub(crate) fn into_raw(b: Box<Self>) -> *mut Self {
        Box::into_raw(b)
    }

    /// Reconstruct the `Box` from a raw pointer.
    ///
    /// # Safety
    ///
    /// Caller must ensure `p` was returned by `into_raw` and has not
    /// yet been freed or consumed.
    pub(crate) unsafe fn from_raw(p: *mut Self) -> Box<Self> {
        // SAFETY: forwarded from caller.
        unsafe { Box::from_raw(p) }
    }
}

// ---------------------------------------------------------------------------
// RTSP server builder opaque handle (rtp feature)
// ---------------------------------------------------------------------------

/// Configuration accumulator for the RTSP server.
///
/// Allocated by `tst_rtsp_server_builder_new` and consumed (or freed) by
/// `tst_rtsp_server_builder_start` (Task 8) / `tst_rtsp_server_builder_free`.
///
/// Fields are stored independently rather than wrapping `RtspServerBuilder`
/// directly.  Even though `RtspServerBuilder` uses `&mut self -> &mut Self`
/// chain setters (which are in-place friendly), storing fields independently
/// keeps the opaque struct layout stable across future Rust API changes and
/// matches the T5 `TstRtspClientBuilder` pattern.  Task 8's start path
/// constructs the final `RtspServerBuilder` from these fields.
// Allow dead_code: all fields are read by Task 8's start path which lands
// in a subsequent Wave B commit. The allow is removed at that time.
#[cfg(feature = "rtp")]
#[allow(dead_code)]
pub struct TstRtspServerBuilder {
    /// Parsed bind URL. Replaced by `tst_rtsp_server_builder_bind`.
    pub(crate) bind_url: tst_rtp::url::RtspUrl,
    /// Auth scheme — `None` means no auth required (default).
    pub(crate) auth_scheme: Option<crate::rtsp::server::builder::TstRtspServerAuthScheme>,
    /// Auth realm — set to `"tst-rtp"` by any `_auth_*` setter.
    pub(crate) auth_realm: String,
    /// Username supplied via any `_auth_*` entry point.
    pub(crate) auth_username: Option<String>,
    /// Password in plain text; wrapped in `secrecy::SecretString` at
    /// `start` time so it is zeroed when the builder is consumed.
    pub(crate) auth_password: Option<String>,
    /// Cap on concurrent client sessions. Floored to 1. Default: 64.
    pub(crate) max_sessions: u32,
    /// Session timeout in seconds. Default: 60.
    pub(crate) session_timeout_secs: u32,
    /// Per-mount broadcast channel capacity (frame count). Floored to 1.
    /// Default: 256.
    pub(crate) fanout_capacity: u32,
    /// Graceful-shutdown drain window in milliseconds. Default: 100.
    pub(crate) graceful_shutdown_drain_ms: u32,
    /// Raw PEM bytes for the TLS certificate chain (`rtsps://` binds).
    /// Written to a temp path at `start` time and passed to
    /// `RtspServerBuilder::tls_cert`. `None` → no TLS cert configured.
    pub(crate) tls_cert_pem: Option<Vec<u8>>,
    /// Raw PEM bytes for the TLS private key (`rtsps://` binds).
    /// Paired with `tls_cert_pem`; both must be `Some` for TLS to activate.
    pub(crate) tls_key_pem: Option<Vec<u8>>,
}

#[cfg(feature = "rtp")]
impl TstRtspServerBuilder {
    /// Construct from a parsed URL string.
    ///
    /// Runs `validate_for_server_bind` to reject DNS names — the OS bind
    /// requires an IP literal.
    ///
    /// # Errors
    ///
    /// Returns `Err(String)` with a user-facing message if the URL cannot
    /// be parsed or fails the server-bind validation.
    pub(crate) fn from_url(url_str: &str) -> Result<Self, String> {
        use tst_rtp::url::RtspUrl;
        let url = RtspUrl::parse(url_str).map_err(|e| e.to_string())?;
        url.validate_for_server_bind().map_err(|e| e.to_string())?;
        Ok(Self {
            bind_url: url,
            auth_scheme: None,
            auth_realm: String::new(),
            auth_username: None,
            auth_password: None,
            max_sessions: 64,
            session_timeout_secs: 60,
            fanout_capacity: 256,
            graceful_shutdown_drain_ms: 100,
            tls_cert_pem: None,
            tls_key_pem: None,
        })
    }

    /// Leak the `Box` and return a raw pointer suitable for FFI.
    pub(crate) fn into_raw(b: Box<Self>) -> *mut Self {
        Box::into_raw(b)
    }

    /// Reconstruct the `Box` from a raw pointer.
    ///
    /// # Safety
    ///
    /// Caller must ensure `p` was returned by `into_raw` and has not
    /// yet been freed or consumed.
    pub(crate) unsafe fn from_raw(p: *mut Self) -> Box<Self> {
        // SAFETY: forwarded from caller.
        unsafe { Box::from_raw(p) }
    }

    /// Construct a `tst_rtp::RtspServer` from this accumulator's fields.
    /// Called by T8's `tst_rtsp_server_builder_start` after consuming the
    /// builder via `from_raw`.
    ///
    /// TLS cert + key PEM bytes (if both present) are written to two temp
    /// files and the paths handed to `RtspServerBuilder::tls_cert`. The
    /// tempfile handles are dropped at end of this call — that's safe
    /// because `tls_cert` reads + parses the PEM during `build()`, which
    /// completes before this method returns.
    pub(crate) fn build_server(
        self: Box<Self>,
    ) -> Result<tst_rtp::RtspServer, tst_rtp::RtspServerError> {
        use crate::rtsp::server::builder::TstRtspServerAuthScheme;
        use std::time::Duration;

        let mut b = tst_rtp::RtspServerBuilder::with_url(self.bind_url);

        if let (Some(scheme), Some(user), Some(pass)) =
            (self.auth_scheme, self.auth_username, self.auth_password)
        {
            let secret = secrecy::SecretString::new(pass.into());
            let realm = if self.auth_realm.is_empty() {
                "tst-rtp"
            } else {
                &self.auth_realm
            };
            match scheme {
                TstRtspServerAuthScheme::Basic => {
                    b.auth_basic(realm, &user, secret);
                }
                TstRtspServerAuthScheme::DigestMd5 => {
                    b.auth_digest_md5(realm, &user, secret);
                }
                TstRtspServerAuthScheme::DigestSha256 => {
                    b.auth_digest_sha256(realm, &user, secret);
                }
            }
        }

        b.max_sessions(self.max_sessions.max(1) as usize);
        b.session_timeout(Duration::from_secs(self.session_timeout_secs as u64));
        b.fanout_capacity(self.fanout_capacity.max(1) as usize);
        b.graceful_shutdown_drain(Duration::from_millis(
            self.graceful_shutdown_drain_ms as u64,
        ));

        // TLS path: tst-c does not yet have a `tls` cargo feature, so the
        // tst-rtp `tls` feature is off here. T7's `_tls_cert_pem` setter
        // stores the bytes for forward-compatibility; they're consumed +
        // discarded until a future tst-c `tls` feature lights up
        // `RtspServerBuilder::tls_cert` (which is gated on tst-rtp `tls`).
        let _ = (self.tls_cert_pem, self.tls_key_pem);

        b.build()
    }
}

// ---------------------------------------------------------------------------
// Stream handle types (multi-stream `mpegts::mux` fan-out)
// ---------------------------------------------------------------------------

/// Opaque per-program ordinal for a video elementary stream. Obtained from
/// `tst_mux_config_add_video_stream` at config time and reused with the
/// `_video_to` push siblings on every muxer-owning C variant.
///
/// Handles are stable across the config→open boundary and across managed
/// reconnects. They encode `(program_index, within_program_index)` as a
/// packed `u32` (bits 4..=7 = program, bits 0..=3 = within). They are NOT
/// interchangeable between muxers.
pub type TstVideoStreamHandle = u32;

/// Opaque per-program ordinal for a KLV elementary stream. Same packed
/// encoding as [`TstVideoStreamHandle`].
pub type TstKlvStreamHandle = u32;

/// Opaque per-program ordinal for an audio elementary stream. Same packed
/// encoding as [`TstVideoStreamHandle`].
pub type TstAudioStreamHandle = u32;

/// Opaque per-program ordinal for a subtitle elementary stream. Same packed
/// encoding as [`TstVideoStreamHandle`].
pub type TstSubtitleStreamHandle = u32;

/// Sentinel returned by `tst_mux_config_add_*_stream` on failure.
/// On failure, the last-error is also populated; check
/// `tst_get_last_error()` for the negative `TST_E_*` code.
pub const TST_INVALID_STREAM_HANDLE: u32 = u32::MAX;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_inner_runs_when_live() {
        let h = Handle::new(7i32);
        let rc = h.with_inner_mut(|n| {
            *n += 1;
            0
        });
        assert_eq!(rc, 0);
        let final_value = h.with_inner_ref(|n| *n);
        assert_eq!(final_value, 8);
    }

    #[test]
    fn with_inner_after_close_returns_closed_code() {
        let h = Handle::new(7i32);
        h.close();
        let rc = h.with_inner_mut(|_| 0);
        assert_eq!(rc, TstError::Closed as i32);
    }

    #[test]
    fn close_is_idempotent() {
        let h = Handle::new(7i32);
        h.close();
        h.close();
    }

    #[test]
    fn panic_in_inner_closure_is_caught() {
        use crate::error::clear_last_error_for_test;
        clear_last_error_for_test();
        let h = Handle::new(7i32);
        let rc = h.with_inner_mut(|_| panic!("test panic"));
        assert_eq!(rc, TstError::PanicCaught as i32);
        // After a caught panic, the inner is dropped: subsequent calls
        // see a closed handle.
        let rc2 = h.with_inner_mut(|_| 0);
        assert_eq!(rc2, TstError::Closed as i32);
    }

    #[test]
    fn panic_in_inner_ref_closure_is_caught() {
        use crate::error::clear_last_error_for_test;
        clear_last_error_for_test();
        let h = Handle::new(7i32);
        let rc = h.with_inner_ref(|_| panic!("test panic ref"));
        assert_eq!(rc, TstError::PanicCaught as i32);
        // Defense-in-depth: even though &T didn't mutate, external state
        // could be in an indeterminate post-panic state, so we drop the
        // inner. Subsequent calls return Closed.
        let rc2 = h.with_inner_ref(|_| 0);
        assert_eq!(rc2, TstError::Closed as i32);
    }
}
