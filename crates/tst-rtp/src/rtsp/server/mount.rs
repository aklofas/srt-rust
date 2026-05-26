//! Mount-handle surface — the public push API exposed to callers via
//! `RtspServer::add_mount` and (Task 14) `add_multicast_mount`.
//!
//! v1 architecture (per sub-design §D1):
//! - One `Muxer` per mount (inside a `Mutex` for sync access from
//!   thread-safe push methods).
//! - One `broadcast::Sender<Bytes>` per mount fanning out TS bytes to
//!   N peers' per-session subscriber tasks (Task 13 wires the
//!   subscriber side).
//! - `MountHandle` carries an `Arc<MountState>` + the registered mount
//!   path string. Cloning the handle is cheap; multiple clones can push
//!   from different threads.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::Mutex;

use bytes::Bytes;
use tokio::sync::broadcast;

use tst_core::mpegts::mux::{Muxer, MuxerConfig};

/// Discriminant for mount type.
///
/// `Unicast` mounts pair the broadcast fanout with per-session
/// subscriber tasks (Task 13 wires the subscriber side); `Multicast`
/// mounts use one shared UDP socket per group (Task 14 wires it).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum MountKind {
    /// Unicast mount — each connected client gets its own per-session
    /// task subscribing to the broadcast.
    Unicast,
    /// Multicast mount — one shared UDP socket sends to the group;
    /// per-session tasks collapse to a counter bump.
    Multicast {
        group: SocketAddr,
        ttl: u8,
        iface: Option<String>,
    },
}

/// Internal per-mount state. Held inside `ServerState::mounts` as
/// `Arc<MountState>`. Public surface is via `MountHandle` only.
///
/// Several fields land in subsequent tasks:
/// - Task 13 (fanout) drives `fanout` subscribers.
/// - Task 14 (multicast) constructs `MountKind::Multicast` variants.
/// - Task 15 (push) makes `muxer` + `stats` write-through.
#[allow(dead_code)]
pub(crate) struct MountState {
    pub(crate) path: String,
    pub(crate) kind: MountKind,
    pub(crate) muxer: Mutex<Muxer>,
    /// Broadcast sender — fanout target for serialized TS bytes. Subscribers
    /// are created by Task 13's per-session subscriber task on PLAY.
    pub(crate) fanout: broadcast::Sender<Bytes>,
    pub(crate) stats: Mutex<MountStatsInner>,
}

/// Internal stats accumulator. Public `MountStats` snapshot derived from this.
#[allow(dead_code)]
#[derive(Default)]
pub(crate) struct MountStatsInner {
    pub(crate) bytes_pushed: u64,
    pub(crate) packets_pushed: u64,
    pub(crate) frames_dropped_total: u64,
    pub(crate) per_stream: BTreeMap<u16, tst_core::mpegts::stats::StreamStats>,
}

impl MountState {
    /// Construct a fresh MountState from a MuxerConfig + MountKind +
    /// fanout capacity. Task 14 routes here for multicast as well; for
    /// now unicast is the only caller (via `RtspServer::add_mount`).
    pub(crate) fn new(
        path: impl Into<String>,
        kind: MountKind,
        muxer_cfg: MuxerConfig,
        fanout_capacity: usize,
    ) -> Result<Arc<Self>, crate::error::RtspServerError> {
        let muxer =
            Muxer::new(muxer_cfg).map_err(|e| crate::error::RtspServerError::InvalidConfig {
                detail: format!("muxer construction failed: {e}"),
            })?;
        let (tx, _rx) = broadcast::channel(fanout_capacity.max(1));
        Ok(Arc::new(Self {
            path: path.into(),
            kind,
            muxer: Mutex::new(muxer),
            fanout: tx,
            stats: Mutex::new(MountStatsInner::default()),
        }))
    }

    /// True if this mount is unicast.
    #[allow(dead_code)]
    pub(crate) fn is_unicast(&self) -> bool {
        matches!(self.kind, MountKind::Unicast)
    }
}

/// Snapshot of [`MountHandle::stats`].
///
/// `bytes_pushed` and `packets_pushed` are cumulative since the mount
/// was added. `peer_count` reflects the live subscriber count on the
/// broadcast channel. `frames_dropped_total` sums per-peer dropped-frame
/// counters reported by lagging subscribers.
#[must_use]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct MountStats {
    pub bytes_pushed: u64,
    pub packets_pushed: u64,
    pub peer_count: usize,
    pub frames_dropped_total: u64,
    pub per_stream: BTreeMap<u16, tst_core::mpegts::stats::StreamStats>,
}

/// Public mount surface. Returned by `RtspServer::add_mount` /
/// `add_multicast_mount`. Cloning is cheap (clones the `Arc`); multiple
/// holders can push from different threads.
///
/// The actual push API (`push_video` / `push_klv` / `push_audio` /
/// `push_subtitle`) lands in Task 15 — this skeleton ships only the
/// surface type + accessors (`mount_path`, `peer_count`, `stats`,
/// `mount_kind`).
#[derive(Clone)]
pub struct MountHandle {
    pub(crate) state: Arc<MountState>,
}

impl std::fmt::Debug for MountHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MountHandle")
            .field("path", &self.state.path)
            .field("kind", &self.state.kind)
            .field("peer_count", &self.state.fanout.receiver_count())
            .finish()
    }
}

impl MountHandle {
    /// The mount path registered via `add_mount("/path", ...)`.
    pub fn mount_path(&self) -> &str {
        &self.state.path
    }

    /// Live subscriber count on the broadcast channel. For unicast
    /// mounts this is the number of currently-PLAYing clients; for
    /// multicast it's typically 1 (the per-mount multicast sender task).
    pub fn peer_count(&self) -> usize {
        self.state.fanout.receiver_count()
    }

    /// Snapshot of cumulative + live mount stats. Mutates nothing.
    pub fn stats(&self) -> MountStats {
        let stats_lock = match self.state.stats.lock() {
            Ok(g) => g,
            Err(_) => return MountStats::default(),
        };
        MountStats {
            bytes_pushed: stats_lock.bytes_pushed,
            packets_pushed: stats_lock.packets_pushed,
            peer_count: self.state.fanout.receiver_count(),
            frames_dropped_total: stats_lock.frames_dropped_total,
            per_stream: stats_lock.per_stream.clone(),
        }
    }

    /// Discriminant for the mount type.
    pub fn mount_kind(&self) -> &MountKind {
        &self.state.kind
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tst_core::mpegts::mux::{MuxerConfig, MuxerProgramConfigBuilder, VideoCodec};

    fn mock_unicast_mount_state() -> Arc<MountState> {
        // Minimal MuxerConfig: one program with H.264 video.
        let mut prog = MuxerProgramConfigBuilder::new(1, 0x1000);
        prog.add_video(0x1011, VideoCodec::H264);
        let mut b = MuxerConfig::builder();
        b.add_program(prog.build());
        let cfg = b.build().unwrap();
        MountState::new("/test", MountKind::Unicast, cfg, 256).unwrap()
    }

    #[test]
    fn mount_state_constructs() {
        let state = mock_unicast_mount_state();
        assert_eq!(state.path, "/test");
        assert!(state.is_unicast());
    }

    #[test]
    fn mount_handle_path() {
        let state = mock_unicast_mount_state();
        let handle = MountHandle { state };
        assert_eq!(handle.mount_path(), "/test");
    }

    #[test]
    fn mount_handle_peer_count_zero_initially() {
        let state = mock_unicast_mount_state();
        let handle = MountHandle { state };
        // No subscribers until Task 13's per-session task subscribes.
        assert_eq!(handle.peer_count(), 0);
    }

    #[test]
    fn mount_handle_stats_default() {
        let state = mock_unicast_mount_state();
        let handle = MountHandle { state };
        let stats = handle.stats();
        assert_eq!(stats.bytes_pushed, 0);
        assert_eq!(stats.packets_pushed, 0);
        assert_eq!(stats.peer_count, 0);
        assert_eq!(stats.frames_dropped_total, 0);
    }

    #[test]
    fn mount_handle_clone_is_cheap() {
        let state = mock_unicast_mount_state();
        let h1 = MountHandle { state };
        let h2 = h1.clone();
        // Both reference the same MountState (Arc count == 2).
        assert!(Arc::ptr_eq(&h1.state, &h2.state));
    }

    #[test]
    fn mount_handle_kind_is_unicast() {
        let state = mock_unicast_mount_state();
        let handle = MountHandle { state };
        assert!(matches!(handle.mount_kind(), MountKind::Unicast));
    }
}
