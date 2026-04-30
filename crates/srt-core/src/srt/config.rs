//! Configuration structs — the canonical types for socket/listener setup.
//!
//! `SocketBuilder`/`ListenerBuilder` (in `builder.rs`) are sugar over these.
//! Bindings (UniFFI dictionaries, JNI POJOs, cbindgen C structs) consume
//! these directly.

use crate::srt::options::{Congestion, KeyLength, MaxBandwidth, PacketFilter, Passphrase, StreamId};
use std::time::Duration;

/// Configuration for a caller-side data socket.
///
/// All `Option<T>` fields default to `None`, meaning "leave the libsrt default."
/// `KeyLength` defaults to `Aes128` (only relevant when `passphrase` is set).
#[derive(Debug, Clone, Default)]
pub struct SocketConfig {
    // Encryption
    pub passphrase: Option<Passphrase>,
    pub key_length: KeyLength,

    // Timeouts
    pub send_timeout: Option<Duration>,
    pub recv_timeout: Option<Duration>,

    // Latency / buffering
    pub latency: Option<Duration>,
    pub peer_latency: Option<Duration>,
    pub recv_latency: Option<Duration>,
    pub recv_buf_packets: Option<u32>,
    pub send_buf_packets: Option<u32>,

    // Bandwidth
    pub max_bandwidth: Option<MaxBandwidth>,
    pub input_bandwidth: Option<u64>,
    pub overhead_bandwidth_pct: Option<u8>,

    // Wire / packet sizing
    pub mss: Option<u16>,
    pub payload_size: Option<u16>,

    // Identification
    pub stream_id: Option<StreamId>,

    // Reliability / loss
    pub loss_max_ttl: Option<u32>,

    // Congestion / priority
    pub too_late_packet_drop: Option<bool>,
    pub flow_window_packets: Option<u32>,
    pub packet_filter: Option<PacketFilter>,
    pub congestion: Option<Congestion>,
}

/// Configuration for a listener (passive) socket.
///
/// Most fields parallel `SocketConfig`; the listener applies them to itself
/// and inherits the timeout fields onto sockets returned by `accept()`.
#[derive(Debug, Clone)]
pub struct ListenerConfig {
    // Encryption (applied to listener; libsrt enforces during handshake)
    pub passphrase: Option<Passphrase>,
    pub key_length: KeyLength,

    // Latency / buffering (listener-side)
    pub latency: Option<Duration>,
    pub recv_latency: Option<Duration>,
    pub recv_buf_packets: Option<u32>,

    // Bandwidth
    pub max_bandwidth: Option<MaxBandwidth>,
    pub overhead_bandwidth_pct: Option<u8>,

    // Wire / packet sizing
    pub mss: Option<u16>,
    pub payload_size: Option<u16>,

    // Reliability / loss
    pub loss_max_ttl: Option<u32>,

    // Congestion / priority
    pub too_late_packet_drop: Option<bool>,
    pub flow_window_packets: Option<u32>,
    pub packet_filter: Option<PacketFilter>,
    pub congestion: Option<Congestion>,

    // Listener-specific
    pub backlog: u32,
    pub reuse_addr: bool,

    // Inherited by accepted sockets
    pub recv_timeout: Option<Duration>,
    pub send_timeout: Option<Duration>,
}

impl Default for ListenerConfig {
    fn default() -> Self {
        Self {
            passphrase: None,
            key_length: KeyLength::default(),
            latency: None,
            recv_latency: None,
            recv_buf_packets: None,
            max_bandwidth: None,
            overhead_bandwidth_pct: None,
            mss: None,
            payload_size: None,
            loss_max_ttl: None,
            too_late_packet_drop: None,
            flow_window_packets: None,
            packet_filter: None,
            congestion: None,
            backlog: 5,
            reuse_addr: true,
            recv_timeout: None,
            send_timeout: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_default_is_unencrypted() {
        let cfg = SocketConfig::default();
        assert!(cfg.passphrase.is_none());
        assert_eq!(cfg.key_length, KeyLength::Aes128);
    }

    #[test]
    fn listener_default_has_backlog_5() {
        let cfg = ListenerConfig::default();
        assert_eq!(cfg.backlog, 5);
        assert!(cfg.reuse_addr);
    }
}
