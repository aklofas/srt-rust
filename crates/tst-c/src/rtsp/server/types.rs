//! Opaque handle types for the RTSP server C ABI.
//!
//! `TstRtspServer` — owns the live `RtspServer` Rust value behind a `Mutex`.
//! `TstRtspMountHandle` — owns a `MountHandle` returned from
//! `RtspServer::add_mount` / `add_multicast_mount`; push methods land in T9.
//!
//! Both types are opaque from the C caller's perspective. The naming follows
//! the `tst_rtsp_server_t` / `tst_rtsp_mount_handle_t` C type names emitted
//! by cbindgen.
//!
//! # Lifecycle
//!
//! ```text
//! tst_rtsp_server_builder_new()  →  TstRtspServerBuilder (Task 7)
//!      ↓  (setter calls — Task 7)
//! tst_rtsp_server_builder_start()  →  TstRtspServer  (Task 8)
//!      ↓
//! tst_rtsp_server_add_unicast_mount()    →  TstRtspMountHandle  (Task 8)
//! tst_rtsp_server_add_multicast_mount()  →  TstRtspMountHandle  (Task 8)
//!      ↓
//! push_video / push_klv / … on TstRtspMountHandle  (Task 9)
//!      ↓
//! tst_rtsp_server_stop() / tst_rtsp_server_free()  (Task 10)
//! ```

use std::sync::Mutex;

/// Opaque handle for a started RTSP server.
///
/// Obtained from [`super::start::tst_rtsp_server_builder_start`]. Freed (with
/// graceful shutdown) via `tst_rtsp_server_stop` + `tst_rtsp_server_free`
/// (Task 10), or implicitly via Drop (hard cancel).
///
/// The inner `Mutex<Option<…>>` gives close-idempotence: after `_stop` or
/// `_free` the `Option` is `None` and subsequent calls return `TST_E_CLOSED`.
/// This mirrors the `TstRtspSession` shape used in the client surface
/// (Task 6).
pub struct TstRtspServer {
    /// Live Rust `RtspServer`. `None` after a call to `tst_rtsp_server_stop`
    /// or `tst_rtsp_server_free` consumes the value.
    pub(crate) inner: Mutex<Option<Box<tst_rtp::RtspServer>>>,
    /// Hard-cancel handle. Cloned from the server before inserting into
    /// `inner` so that `tst_rtsp_server_cancel` (Task 10) can fire it
    /// without acquiring the `inner` Mutex.
    // `cancel` is read by Task 10's `tst_rtsp_server_cancel` entry point.
    // Allow dead_code until T10 lands.
    #[allow(dead_code)]
    pub(crate) cancel: tst_rtp::RtspServerCancelHandle,
}

impl TstRtspServer {
    /// Wrap a `RtspServer` in an opaque handle, extracting the cancel handle.
    pub(crate) fn new(server: tst_rtp::RtspServer) -> Self {
        let cancel = server.cancel_handle();
        Self {
            inner: Mutex::new(Some(Box::new(server))),
            cancel,
        }
    }
}

/// Opaque handle for an RTSP mount.
///
/// Obtained from [`super::mount::tst_rtsp_server_add_unicast_mount`] or
/// [`super::mount::tst_rtsp_server_add_multicast_mount`]. Push methods
/// (`push_video`, `push_klv`, etc.) land in Task 9. Freed with
/// `tst_rtsp_mount_handle_free` (Task 9 or 10 scope).
///
/// The `MountHandle` returned by the Rust API is `Clone + Send`, so multiple
/// C handles pointing at the same mount are safe — each clone pushes to the
/// same broadcast fanout channel.
pub struct TstRtspMountHandle {
    /// The inner `MountHandle`. T9 adds push methods that delegate to
    /// `MountHandle::send_video` / `send_klv` / etc.
    // Allow dead_code until T9 lands push methods that access this field.
    #[allow(dead_code)]
    pub(crate) inner: tst_rtp::MountHandle,
}
