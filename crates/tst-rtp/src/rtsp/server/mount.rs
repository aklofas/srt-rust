//! Mount surface — placeholder for Wave C Tasks 12-15.

use std::net::SocketAddr;

/// Discriminant for mount type. Wave C extends with the broadcast
/// channel + per-mount Muxer.
#[derive(Debug, Clone)]
#[non_exhaustive]
#[allow(dead_code)]
pub(crate) enum MountKind {
    Unicast,
    Multicast {
        group: SocketAddr,
        ttl: u8,
        iface: Option<String>,
    },
}

/// Minimal placeholder so the server-state hashmap typechecks. Wave C
/// (Tasks 12+) replaces with the full struct holding Muxer + broadcast.
#[allow(dead_code)]
pub(crate) struct MountState {
    pub(crate) kind: MountKind,
}
