//! `tst_rtsp_server_add_unicast_mount` and `tst_rtsp_server_add_multicast_mount`.
//!
//! Both entry points register a new mount on a started `TstRtspServer` and
//! return an opaque `TstRtspMountHandle*`. Push methods on the mount handle
//! land in Task 9; stats, stop, and server-level free land in Task 10.
//!
//! # Mount registration pattern
//!
//! ```text
//! tst_rtsp_server_add_unicast_mount(server, path, mux_cfg)
//!     │
//!     ├─ validate inputs (null check, UTF-8 decode)
//!     ├─ build MuxerConfig from TstMuxConfig  ← same as tst_rtp_mux_sender_open
//!     ├─ call RtspServer::add_mount(path, cfg)  ← inside inner Mutex
//!     └─ return Box<TstRtspMountHandle>::into_raw(…)
//!
//! tst_rtsp_server_add_multicast_mount(server, path, group, ttl, iface, mux_cfg)
//!     │
//!     ├─ validate inputs
//!     ├─ build group_url string: "rtp://<group>:<port>?ttl=N[&iface=X]"
//!     │   (only ttl and optional iface needed; port is embedded in group arg)
//!     ├─ build MuxerConfig
//!     ├─ call RtspServer::add_multicast_mount(path, cfg, group_url)
//!     └─ return Box<TstRtspMountHandle>::into_raw(…)
//! ```
//!
//! # Group URL format for multicast
//!
//! `RtspServer::add_multicast_mount` expects a `group_url` of the form
//! `rtp://<multicast-ip>:<port>[?ttl=N&iface=name_or_ip]`. The C entry point
//! accepts `group` (NUL-terminated, must be `<ip>:<port>` like
//! `"239.0.0.1:5004"`), `ttl` (u8), and `iface_name` (NUL-terminated, may
//! be NULL for default). The group URL string is assembled here and forwarded
//! to `add_multicast_mount`.
//!
//! # Error mapping
//!
//! - `RtspServerError` variants → `TST_E_RTSP_SERVER` (-24) via
//!   [`crate::error::rtsp_server_error_to_code`].
//! - `MuxError` from config validation → mapped via
//!   [`crate::error::record_mux_error`].

use std::ffi::CStr;
use std::os::raw::c_char;

use crate::config::TstMuxConfig;
use crate::error::{TstError, record_mux_error, rtsp_server_error_to_code, set_last_error};
use crate::panic::ffi_catch;
use crate::rtsp::server::types::{TstRtspMountHandle, TstRtspServer};

/// Register a **unicast** mount on a started RTSP server.
///
/// After a successful call, connecting clients that send an RTSP SETUP
/// request to `path` will be assigned individual UDP or TCP-interleaved
/// transports. Each client's RTP stream is fed from the same broadcast
/// fanout channel that the returned [`TstRtspMountHandle`] writes into.
///
/// `path` must:
/// - Be a NUL-terminated UTF-8 string.
/// - Start with `/` (e.g. `"/live"`).
/// - Not contain URL-reserved characters like `?` or `#`.
/// - Not duplicate a path already registered on this server.
///
/// `mux_cfg` must be a valid `tst_mux_config_t` (see `tst_mux_config_new`
/// / `tst_mux_config_add_program`). It is borrowed for this call — the
/// caller still owns it and must free it. The returned handle is independent
/// of the config after this call.
///
/// Returns a non-NULL `tst_rtsp_mount_handle_t*` on success, NULL on
/// failure with last-error set. The handle must eventually be freed with
/// `tst_rtsp_mount_handle_free` (Task 9 / 10 scope).
///
/// # Safety
///
/// - `server` must be a non-NULL, non-freed pointer from
///   `tst_rtsp_server_builder_start`.
/// - `path` must be a NUL-terminated C string valid for this call.
/// - `mux_cfg` must be a non-NULL pointer from `tst_mux_config_new`,
///   valid for this call (caller retains ownership).
#[cfg(feature = "rtp")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_rtsp_server_add_unicast_mount(
    server: *mut TstRtspServer,
    path: *const c_char,
    mux_cfg: *const TstMuxConfig,
) -> *mut TstRtspMountHandle {
    ffi_catch(std::ptr::null_mut(), || {
        // Validate server pointer.
        let Some(handle) = (unsafe { server.as_ref() }) else {
            set_last_error(TstError::InvalidConfig, "server is null");
            return std::ptr::null_mut();
        };

        // Decode the mount path.
        if path.is_null() {
            set_last_error(TstError::InvalidConfig, "path is null");
            return std::ptr::null_mut();
        }
        let path_str = match unsafe { CStr::from_ptr(path) }.to_str() {
            Ok(s) => s,
            Err(_) => {
                set_last_error(TstError::InvalidConfig, "path is not valid UTF-8");
                return std::ptr::null_mut();
            }
        };

        // Build MuxerConfig from the TstMuxConfig builder.
        let cfg_ref = match unsafe { mux_cfg.as_ref() } {
            Some(c) => c,
            None => {
                set_last_error(TstError::InvalidConfig, "mux_cfg is null");
                return std::ptr::null_mut();
            }
        };
        let muxer_cfg = match cfg_ref.build_config() {
            Ok(c) => c,
            Err(e) => {
                record_mux_error(&e);
                return std::ptr::null_mut();
            }
        };

        // Lock the server and call add_mount.
        let guard = match handle.inner.lock() {
            Ok(g) => g,
            Err(_) => {
                set_last_error(TstError::Internal, "server mutex poisoned");
                return std::ptr::null_mut();
            }
        };
        let server_ref = match guard.as_ref() {
            Some(s) => s,
            None => {
                set_last_error(TstError::Closed, "server is stopped or freed");
                return std::ptr::null_mut();
            }
        };

        match server_ref.add_mount(path_str, muxer_cfg) {
            Ok(mount) => Box::into_raw(Box::new(TstRtspMountHandle { inner: mount })),
            Err(e) => {
                let code = rtsp_server_error_to_code(&e);
                set_last_error(code, &format!("add_unicast_mount failed: {e}"));
                std::ptr::null_mut()
            }
        }
    })
}

/// Register a **multicast** mount on a started RTSP server.
///
/// After a successful call, the server spawns a background task that drains
/// the mount's broadcast channel and sends RTP packets to the multicast
/// `group` address. Connecting clients that SETUP against `path` receive the
/// same group address, TTL, and optional interface in the `Transport:` header
/// so they can join the group and receive the shared stream.
///
/// `group` must be a NUL-terminated `<ip>:<port>` string (IPv4 or IPv6
/// multicast address with port), e.g. `"239.0.0.1:5004"`. The address must
/// be in a multicast range (IPv4 `224.0.0.0/4`, IPv6 `ff00::/8`). Port must
/// be included.
///
/// `ttl` is the IP multicast TTL / hop limit (1–255). Typical LAN values:
/// 1 = link-local, 8 = site-local (RTP convention).
///
/// `iface_name` is a NUL-terminated string identifying the outbound interface
/// for multicast send — either an IPv4 literal (e.g. `"192.168.1.50"`) or an
/// interface name where supported. Pass NULL to let the OS select the default
/// multicast interface.
///
/// `mux_cfg` is borrowed — the caller retains ownership and must free it.
///
/// Returns a non-NULL `tst_rtsp_mount_handle_t*` on success, NULL on failure
/// with last-error set. NULL for `iface_name` is valid (no interface
/// override).
///
/// # Safety
///
/// - `server` must be a non-NULL, non-freed pointer from
///   `tst_rtsp_server_builder_start`.
/// - `path` must be a NUL-terminated C string valid for this call.
/// - `group` must be a NUL-terminated `<ip>:<port>` string.
/// - `iface_name` may be NULL or a NUL-terminated interface string.
/// - `mux_cfg` must be a non-NULL pointer from `tst_mux_config_new`,
///   valid for this call.
#[cfg(feature = "rtp")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_rtsp_server_add_multicast_mount(
    server: *mut TstRtspServer,
    path: *const c_char,
    group: *const c_char,
    ttl: u8,
    iface_name: *const c_char,
    mux_cfg: *const TstMuxConfig,
) -> *mut TstRtspMountHandle {
    ffi_catch(std::ptr::null_mut(), || {
        // Validate server pointer.
        let Some(handle) = (unsafe { server.as_ref() }) else {
            set_last_error(TstError::InvalidConfig, "server is null");
            return std::ptr::null_mut();
        };

        // Decode mount path.
        if path.is_null() {
            set_last_error(TstError::InvalidConfig, "path is null");
            return std::ptr::null_mut();
        }
        let path_str = match unsafe { CStr::from_ptr(path) }.to_str() {
            Ok(s) => s,
            Err(_) => {
                set_last_error(TstError::InvalidConfig, "path is not valid UTF-8");
                return std::ptr::null_mut();
            }
        };

        // Decode group address.
        if group.is_null() {
            set_last_error(TstError::InvalidConfig, "group is null");
            return std::ptr::null_mut();
        }
        let group_str = match unsafe { CStr::from_ptr(group) }.to_str() {
            Ok(s) => s,
            Err(_) => {
                set_last_error(TstError::InvalidConfig, "group is not valid UTF-8");
                return std::ptr::null_mut();
            }
        };

        // Decode optional iface_name.
        let iface_str: Option<&str> = if iface_name.is_null() {
            None
        } else {
            match unsafe { CStr::from_ptr(iface_name) }.to_str() {
                Ok(s) => Some(s),
                Err(_) => {
                    set_last_error(TstError::InvalidConfig, "iface_name is not valid UTF-8");
                    return std::ptr::null_mut();
                }
            }
        };

        // Assemble the group URL that RtspServer::add_multicast_mount expects:
        // "rtp://<group_str>[?ttl=N][&iface=X]"
        // group_str is already "<ip>:<port>", so we prepend "rtp://".
        let mut group_url = format!("rtp://{group_str}?ttl={ttl}");
        if let Some(iface) = iface_str {
            group_url.push_str("&iface=");
            group_url.push_str(iface);
        }

        // Build MuxerConfig.
        let cfg_ref = match unsafe { mux_cfg.as_ref() } {
            Some(c) => c,
            None => {
                set_last_error(TstError::InvalidConfig, "mux_cfg is null");
                return std::ptr::null_mut();
            }
        };
        let muxer_cfg = match cfg_ref.build_config() {
            Ok(c) => c,
            Err(e) => {
                record_mux_error(&e);
                return std::ptr::null_mut();
            }
        };

        // Lock the server and call add_multicast_mount.
        let guard = match handle.inner.lock() {
            Ok(g) => g,
            Err(_) => {
                set_last_error(TstError::Internal, "server mutex poisoned");
                return std::ptr::null_mut();
            }
        };
        let server_ref = match guard.as_ref() {
            Some(s) => s,
            None => {
                set_last_error(TstError::Closed, "server is stopped or freed");
                return std::ptr::null_mut();
            }
        };

        match server_ref.add_multicast_mount(path_str, muxer_cfg, &group_url) {
            Ok(mount) => Box::into_raw(Box::new(TstRtspMountHandle { inner: mount })),
            Err(e) => {
                let code = rtsp_server_error_to_code(&e);
                set_last_error(code, &format!("add_multicast_mount failed: {e}"));
                std::ptr::null_mut()
            }
        }
    })
}

/// Free an RTSP mount handle.
///
/// Drops the `TstRtspMountHandle` and its inner `MountHandle`. After this
/// call the pointer is invalid; any further use is undefined behavior. NULL
/// is a no-op.
///
/// Push methods on a freed handle are not safe — the caller must not call
/// any `tst_rtsp_mount_handle_*` push method after `_free`.
///
/// # Safety
///
/// `handle` must be NULL, or a pointer returned by
/// `tst_rtsp_server_add_unicast_mount` / `tst_rtsp_server_add_multicast_mount`
/// that has not yet been freed.
#[cfg(feature = "rtp")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_rtsp_mount_handle_free(handle: *mut TstRtspMountHandle) {
    ffi_catch((), || {
        if handle.is_null() {
            return;
        }
        // SAFETY: caller guarantees valid, unaliased, un-freed pointer.
        let _ = unsafe { Box::from_raw(handle) };
        // Box drops at end of scope, running MountHandle's Drop.
    });
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handle::TstRtspServerBuilder;

    /// Helper: start a server on 127.0.0.1:0 and return the raw handle.
    fn start_test_server() -> *mut TstRtspServer {
        let b = TstRtspServerBuilder::from_url("rtsp://127.0.0.1:0");
        let raw_b = TstRtspServerBuilder::into_raw(Box::new(b));
        unsafe { crate::rtsp::server::start::tst_rtsp_server_builder_start(raw_b) }
    }

    /// Helper: build a minimal TstMuxConfig with one H.264 video stream.
    fn make_mux_cfg() -> *mut TstMuxConfig {
        use crate::config::streams::TstVideoCodec;
        unsafe {
            let p = crate::config::tst_mux_config_new();
            let prog = crate::config::tst_mux_config_add_program(p, 1, 0x1000);
            let _v = crate::config::streams::tst_mux_config_add_video_stream(
                p,
                prog,
                0x1011,
                TstVideoCodec::H264,
            );
            p
        }
    }

    #[test]
    fn null_server_add_unicast_returns_null() {
        let cfg = make_mux_cfg();
        let path = std::ffi::CString::new("/live").unwrap();
        let h = unsafe {
            tst_rtsp_server_add_unicast_mount(std::ptr::null_mut(), path.as_ptr(), cfg as *const _)
        };
        assert!(h.is_null());
        unsafe { crate::config::tst_mux_config_free(cfg) };
    }

    #[test]
    fn null_path_add_unicast_returns_null() {
        let server = start_test_server();
        assert!(!server.is_null());
        let cfg = make_mux_cfg();
        let h =
            unsafe { tst_rtsp_server_add_unicast_mount(server, std::ptr::null(), cfg as *const _) };
        assert!(h.is_null());
        unsafe { crate::config::tst_mux_config_free(cfg) };
        // Drop server directly (Task 10 adds tst_rtsp_server_free).
        unsafe {
            let _ = Box::from_raw(server);
        }
    }

    #[test]
    fn null_mux_cfg_add_unicast_returns_null() {
        let server = start_test_server();
        assert!(!server.is_null());
        let path = std::ffi::CString::new("/live").unwrap();
        let h =
            unsafe { tst_rtsp_server_add_unicast_mount(server, path.as_ptr(), std::ptr::null()) };
        assert!(h.is_null());
        unsafe {
            let _ = Box::from_raw(server);
        }
    }

    #[test]
    fn valid_unicast_mount_returns_non_null_handle() {
        let server = start_test_server();
        assert!(!server.is_null());
        let cfg = make_mux_cfg();
        let path = std::ffi::CString::new("/live").unwrap();
        let mount =
            unsafe { tst_rtsp_server_add_unicast_mount(server, path.as_ptr(), cfg as *const _) };
        assert!(
            !mount.is_null(),
            "unicast mount returned null; error: {}",
            {
                let s = unsafe { std::ffi::CStr::from_ptr(crate::error::tst_get_last_error_str()) };
                s.to_str().unwrap_or("<invalid utf8>")
            }
        );
        unsafe { tst_rtsp_mount_handle_free(mount) };
        unsafe { crate::config::tst_mux_config_free(cfg) };
        unsafe {
            let _ = Box::from_raw(server);
        }
    }

    #[test]
    fn null_server_add_multicast_returns_null() {
        let cfg = make_mux_cfg();
        let path = std::ffi::CString::new("/mc").unwrap();
        let group = std::ffi::CString::new("239.0.0.1:5004").unwrap();
        let h = unsafe {
            tst_rtsp_server_add_multicast_mount(
                std::ptr::null_mut(),
                path.as_ptr(),
                group.as_ptr(),
                8,
                std::ptr::null(),
                cfg as *const _,
            )
        };
        assert!(h.is_null());
        unsafe { crate::config::tst_mux_config_free(cfg) };
    }

    #[test]
    fn valid_multicast_mount_returns_non_null_handle() {
        let server = start_test_server();
        assert!(!server.is_null());
        let cfg = make_mux_cfg();
        let path = std::ffi::CString::new("/mc").unwrap();
        let group = std::ffi::CString::new("239.0.0.1:5004").unwrap();
        let mount = unsafe {
            tst_rtsp_server_add_multicast_mount(
                server,
                path.as_ptr(),
                group.as_ptr(),
                8,
                std::ptr::null(),
                cfg as *const _,
            )
        };
        assert!(
            !mount.is_null(),
            "multicast mount returned null; error: {}",
            {
                let s = unsafe { std::ffi::CStr::from_ptr(crate::error::tst_get_last_error_str()) };
                s.to_str().unwrap_or("<invalid utf8>")
            }
        );
        unsafe { tst_rtsp_mount_handle_free(mount) };
        unsafe { crate::config::tst_mux_config_free(cfg) };
        unsafe {
            let _ = Box::from_raw(server);
        }
    }

    #[test]
    fn unicast_null_is_noop() {
        unsafe { tst_rtsp_mount_handle_free(std::ptr::null_mut()) };
    }

    #[test]
    fn duplicate_mount_path_returns_null() {
        let server = start_test_server();
        assert!(!server.is_null());
        let cfg1 = make_mux_cfg();
        let cfg2 = make_mux_cfg();
        let path = std::ffi::CString::new("/live").unwrap();
        let m1 =
            unsafe { tst_rtsp_server_add_unicast_mount(server, path.as_ptr(), cfg1 as *const _) };
        assert!(!m1.is_null());
        let m2 =
            unsafe { tst_rtsp_server_add_unicast_mount(server, path.as_ptr(), cfg2 as *const _) };
        assert!(m2.is_null(), "duplicate path should return null");
        unsafe { tst_rtsp_mount_handle_free(m1) };
        unsafe { crate::config::tst_mux_config_free(cfg1) };
        unsafe { crate::config::tst_mux_config_free(cfg2) };
        unsafe {
            let _ = Box::from_raw(server);
        }
    }
}
