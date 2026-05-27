//! Socket configuration knobs for TCP transports.

use std::time::Duration;

use crate::url::TcpUrl;

/// Per-transport socket knobs.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct SocketConfig {
    /// TCP_NODELAY. `None` = OS default; common preference is `Some(true)`
    /// for low-latency streaming (disables Nagle's algorithm).
    pub nodelay: Option<bool>,
    /// SO_KEEPALIVE idle time. `None` = disabled.
    pub keepalive: Option<Duration>,
    /// SO_RCVBUF size in bytes.
    pub rcvbuf: Option<usize>,
    /// SO_SNDBUF size in bytes.
    pub sndbuf: Option<usize>,
    /// Connect timeout (caller-side only).
    pub connect_timeout: Option<Duration>,
    /// Send-side payload chunk size; per-call max for `send_bytes`.
    /// Default 64 KiB (matches reasonable TCP send-buffer sizing).
    pub pkt_size: Option<usize>,
}

impl SocketConfig {
    /// Default per-call payload limit. TCP is a bytestream, so this is
    /// just an upper bound on how much we accept per `send_bytes` call.
    pub const DEFAULT_PKT_SIZE: usize = 64 * 1024;

    pub fn pkt_size_or_default(&self) -> usize {
        self.pkt_size.unwrap_or(Self::DEFAULT_PKT_SIZE)
    }

    pub fn connect_timeout_or_default(&self) -> Duration {
        self.connect_timeout.unwrap_or(Duration::from_secs(10))
    }

    pub fn merge_from_url(&mut self, url: &TcpUrl) {
        if let Some(v) = url.nodelay {
            self.nodelay = Some(v);
        }
        if let Some(v) = url.keepalive {
            self.keepalive = Some(v);
        }
        if let Some(v) = url.rcvbuf {
            self.rcvbuf = Some(v);
        }
        if let Some(v) = url.sndbuf {
            self.sndbuf = Some(v);
        }
        if let Some(v) = url.connect_timeout {
            self.connect_timeout = Some(v);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::url::TcpUrl;

    #[test]
    fn defaults_are_sensible() {
        let cfg = SocketConfig::default();
        assert_eq!(cfg.pkt_size_or_default(), 64 * 1024);
        assert_eq!(cfg.connect_timeout_or_default(), Duration::from_secs(10));
    }

    #[test]
    fn merge_from_url_applies_set_fields_only() {
        let mut cfg = SocketConfig {
            pkt_size: Some(8192),
            ..Default::default()
        };
        let u = TcpUrl::parse("tcp://1.2.3.4:7001?nodelay=1&keepalive=30").unwrap();
        cfg.merge_from_url(&u);
        assert_eq!(cfg.nodelay, Some(true));
        assert_eq!(cfg.keepalive, Some(Duration::from_secs(30)));
        assert_eq!(cfg.pkt_size, Some(8192));
    }
}
