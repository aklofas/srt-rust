//! Fluent builders over `SocketConfig` / `ListenerConfig`.
//!
//! The builder is sugar; the canonical type is the config struct.
//! Bindings consume the config struct directly. Rust callers can use either.

use crate::error::{BindError, ConnectError, OptionError, StreamIdError};
use crate::srt::config::{ListenerConfig, SocketConfig};
use crate::srt::listener::Listener;
use crate::srt::options::{
    Congestion, KeyLength, MaxBandwidth, PacketFilter, Passphrase, StreamId,
};
use crate::srt::socket::Socket;
use std::net::ToSocketAddrs;
use std::time::Duration;

#[derive(Default)]
pub struct SocketBuilder {
    config: SocketConfig,
}

impl SocketBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn passphrase(mut self, p: Passphrase) -> Self {
        self.config.passphrase = Some(p);
        self
    }
    pub fn key_length(mut self, kl: KeyLength) -> Self {
        self.config.key_length = kl;
        self
    }
    pub fn send_timeout(mut self, t: Duration) -> Self {
        self.config.send_timeout = Some(t);
        self
    }
    pub fn recv_timeout(mut self, t: Duration) -> Self {
        self.config.recv_timeout = Some(t);
        self
    }
    pub fn latency(mut self, d: Duration) -> Self {
        self.config.latency = Some(d);
        self
    }
    pub fn latency_ms(self, ms: u64) -> Self {
        self.latency(Duration::from_millis(ms))
    }
    pub fn peer_latency(mut self, d: Duration) -> Self {
        self.config.peer_latency = Some(d);
        self
    }
    pub fn recv_latency(mut self, d: Duration) -> Self {
        self.config.recv_latency = Some(d);
        self
    }
    pub fn mss(mut self, mss: u16) -> Self {
        self.config.mss = Some(mss);
        self
    }
    pub fn payload_size(mut self, n: u16) -> Self {
        self.config.payload_size = Some(n);
        self
    }
    pub fn max_bandwidth(mut self, bw: MaxBandwidth) -> Self {
        self.config.max_bandwidth = Some(bw);
        self
    }
    pub fn input_bandwidth(mut self, bw: u64) -> Self {
        self.config.input_bandwidth = Some(bw);
        self
    }
    pub fn overhead_bandwidth_pct(mut self, pct: u8) -> Self {
        self.config.overhead_bandwidth_pct = Some(pct);
        self
    }
    pub fn recv_buf_packets(mut self, n: u32) -> Self {
        self.config.recv_buf_packets = Some(n);
        self
    }
    pub fn send_buf_packets(mut self, n: u32) -> Self {
        self.config.send_buf_packets = Some(n);
        self
    }
    pub fn stream_id(mut self, id: StreamId) -> Self {
        self.config.stream_id = Some(id);
        self
    }
    /// Convenience: validate-and-set from `&str` / `String`. Returns error if invalid.
    pub fn try_stream_id(
        mut self,
        id: impl TryInto<StreamId, Error = StreamIdError>,
    ) -> Result<Self, OptionError> {
        self.config.stream_id = Some(id.try_into()?);
        Ok(self)
    }
    pub fn loss_max_ttl(mut self, n: u32) -> Self {
        self.config.loss_max_ttl = Some(n);
        self
    }
    pub fn too_late_packet_drop(mut self, on: bool) -> Self {
        self.config.too_late_packet_drop = Some(on);
        self
    }
    pub fn flow_window_packets(mut self, n: u32) -> Self {
        self.config.flow_window_packets = Some(n);
        self
    }
    pub fn packet_filter(mut self, pf: PacketFilter) -> Self {
        self.config.packet_filter = Some(pf);
        self
    }
    pub fn congestion(mut self, c: Congestion) -> Self {
        self.config.congestion = Some(c);
        self
    }

    /// Reach the underlying config (for inspection, copying, FFI marshaling).
    pub fn config(self) -> SocketConfig {
        self.config
    }

    /// Terminal call: open and connect the socket.
    pub fn connect(self, addr: impl ToSocketAddrs) -> Result<Socket, ConnectError> {
        Socket::connect_with(&self.config, addr)
    }
}

#[derive(Default)]
pub struct ListenerBuilder {
    config: ListenerConfig,
}

impl ListenerBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn passphrase(mut self, p: Passphrase) -> Self {
        self.config.passphrase = Some(p);
        self
    }
    pub fn key_length(mut self, kl: KeyLength) -> Self {
        self.config.key_length = kl;
        self
    }
    pub fn latency(mut self, d: Duration) -> Self {
        self.config.latency = Some(d);
        self
    }
    pub fn latency_ms(self, ms: u64) -> Self {
        self.latency(Duration::from_millis(ms))
    }
    pub fn recv_latency(mut self, d: Duration) -> Self {
        self.config.recv_latency = Some(d);
        self
    }
    pub fn mss(mut self, mss: u16) -> Self {
        self.config.mss = Some(mss);
        self
    }
    pub fn payload_size(mut self, n: u16) -> Self {
        self.config.payload_size = Some(n);
        self
    }
    pub fn max_bandwidth(mut self, bw: MaxBandwidth) -> Self {
        self.config.max_bandwidth = Some(bw);
        self
    }
    pub fn overhead_bandwidth_pct(mut self, pct: u8) -> Self {
        self.config.overhead_bandwidth_pct = Some(pct);
        self
    }
    pub fn recv_buf_packets(mut self, n: u32) -> Self {
        self.config.recv_buf_packets = Some(n);
        self
    }
    pub fn loss_max_ttl(mut self, n: u32) -> Self {
        self.config.loss_max_ttl = Some(n);
        self
    }
    pub fn too_late_packet_drop(mut self, on: bool) -> Self {
        self.config.too_late_packet_drop = Some(on);
        self
    }
    pub fn flow_window_packets(mut self, n: u32) -> Self {
        self.config.flow_window_packets = Some(n);
        self
    }
    pub fn packet_filter(mut self, pf: PacketFilter) -> Self {
        self.config.packet_filter = Some(pf);
        self
    }
    pub fn congestion(mut self, c: Congestion) -> Self {
        self.config.congestion = Some(c);
        self
    }
    pub fn backlog(mut self, n: u32) -> Self {
        self.config.backlog = n;
        self
    }
    pub fn reuse_addr(mut self, on: bool) -> Self {
        self.config.reuse_addr = on;
        self
    }
    pub fn recv_timeout(mut self, t: Duration) -> Self {
        self.config.recv_timeout = Some(t);
        self
    }
    pub fn send_timeout(mut self, t: Duration) -> Self {
        self.config.send_timeout = Some(t);
        self
    }

    pub fn config(self) -> ListenerConfig {
        self.config
    }

    /// Terminal call: bind, listen, return the Listener.
    pub fn bind(self, addr: impl ToSocketAddrs) -> Result<Listener, BindError> {
        Listener::bind_with(&self.config, addr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_builder_chains() {
        let cfg = SocketBuilder::new()
            .latency_ms(120)
            .mss(1316)
            .payload_size(1316)
            .config();

        assert_eq!(cfg.latency, Some(Duration::from_millis(120)));
        assert_eq!(cfg.mss, Some(1316));
        assert_eq!(cfg.payload_size, Some(1316));
    }

    #[test]
    fn listener_builder_chains() {
        let cfg = ListenerBuilder::new()
            .backlog(10)
            .reuse_addr(false)
            .config();

        assert_eq!(cfg.backlog, 10);
        assert!(!cfg.reuse_addr);
    }

    #[test]
    fn try_stream_id_validates() {
        let result = SocketBuilder::new().try_stream_id("publish:cam1");
        assert!(result.is_ok());

        let too_long = "a".repeat(513);
        let result = SocketBuilder::new().try_stream_id(too_long.as_str());
        assert!(result.is_err());
    }
}
