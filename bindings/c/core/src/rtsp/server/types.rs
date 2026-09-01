//! Opaque handle types for the RTSP server C ABI.
//!
//! `TstRtspServer` — owns the live `RtspServer` Rust value behind a `Mutex`.
//! `TstRtspMountHandle` — owns a `MountHandle` returned from
//! `RtspServer::add_mount` / `add_multicast_mount`; push methods live in
//! `mount.rs`.
//!
//! Both types are opaque from the C caller's perspective. The naming follows
//! the `tst_rtsp_server_t` / `tst_rtsp_mount_handle_t` C type names emitted
//! by cbindgen.
//!
//! # Lifecycle
//!
//! ```text
//! tst_rtsp_server_builder_new()  →  TstRtspServerBuilder
//!      ↓  (setter calls)
//! tst_rtsp_server_builder_start()  →  TstRtspServer
//!      ↓
//! tst_rtsp_server_add_unicast_mount()    →  TstRtspMountHandle
//! tst_rtsp_server_add_multicast_mount()  →  TstRtspMountHandle
//!      ↓
//! push_video / push_klv / … on TstRtspMountHandle
//!      ↓
//! tst_rtsp_server_stop() / tst_rtsp_server_free()
//! ```

use std::sync::Mutex;
use std::sync::atomic::AtomicBool;

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
    /// `inner` so that `tst_rtsp_server_cancel_handle` can fire it
    /// without acquiring the `inner` Mutex.
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
/// (`push_video`, `push_klv`, etc.) are in Task 9. Freed with
/// `tst_rtsp_mount_handle_free`.
///
/// The `MountHandle` returned by the Rust API is `Clone + Send`, so multiple
/// C handles pointing at the same mount are safe — each clone pushes to the
/// same broadcast fanout channel.
///
/// The `cancelled` flag is C-layer-only: `tst_rtsp_mount_cancel` sets it and
/// subsequent push calls return `TST_E_CLOSED` immediately without entering
/// the Rust muxer. This avoids the need for a cancel-token in the Rust
/// `MountHandle` API. Unlike transport-based handles, "cancelling" a mount
/// handle only stops this particular C-side caller; the underlying
/// `tst_rtp::MountHandle` (and any other C clones sharing the same broadcast
/// Arc) continues operating.
pub struct TstRtspMountHandle {
    /// The inner `MountHandle`.
    pub(crate) inner: tst_rtp::MountHandle,
    /// Set by `tst_rtsp_mount_cancel`. Guards all push calls — returns
    /// `TST_E_CLOSED` when true. Stored here (not in `MountState`) so that
    /// multiple independent C-side mount handles can have independent cancel
    /// states. Safe to read without the muxer lock because it is checked
    /// before the push path acquires any lock.
    pub(crate) cancelled: AtomicBool,
}
