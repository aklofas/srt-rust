//! Builder types for [`TcpTransport`] and [`TcpListener`].

use std::time::Duration;

use crate::config::SocketConfig;
use crate::error::TcpError;
use crate::listener::TcpListener;
use crate::transport::TcpTransport;
use crate::url::{TcpUrl, TcpUrlError};

/// Builder for caller-side [`TcpTransport`].
///
/// Construct via [`TcpTransportBuilder::from_url`], chain config methods,
/// then call [`build`](Self::build) to establish the connection.
///
/// # Example
///
/// ```no_run
/// # use tst_tcp::builder::TcpTransportBuilder;
/// # use std::time::Duration;
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let mut b = TcpTransportBuilder::from_url("tcp://192.0.2.1:5000")?;
/// b.nodelay(true)
///     .keepalive(Duration::from_secs(30))
///     .rcvbuf(8 * 1024 * 1024);
/// let transport = b.build()?;
/// # Ok(())
/// # }
/// ```
#[must_use]
#[derive(Debug, Clone)]
pub struct TcpTransportBuilder {
    url: TcpUrl,
    config: SocketConfig,
}

impl TcpTransportBuilder {
    /// Create a builder from a `tcp://` or `tcps://` URL.
    ///
    /// # Errors
    ///
    /// Returns [`TcpUrlError`] if the URL is malformed or uses `?listen=1`
    /// (listeners must use [`TcpListenerBuilder`] instead).
    pub fn from_url(url: &str) -> Result<Self, TcpUrlError> {
        let url = TcpUrl::parse(url)?;
        let mut config = SocketConfig::default();
        config.merge_from_url(&url);
        Ok(Self { url, config })
    }

    /// Enable or disable TCP_NODELAY (Nagle's algorithm).
    ///
    /// `true` is typically preferred for low-latency streaming.
    /// Default (if not set): OS-dependent.
    pub fn nodelay(&mut self, on: bool) -> &mut Self {
        self.config.nodelay = Some(on);
        self
    }

    /// Set SO_KEEPALIVE idle timeout.
    ///
    /// Enables TCP keepalive with the given idle duration.
    /// To disable keepalive, do not call this method (it defaults to disabled).
    /// Default (if not set): keepalive disabled.
    pub fn keepalive(&mut self, idle: Duration) -> &mut Self {
        self.config.keepalive = Some(idle);
        self
    }

    /// Set SO_RCVBUF socket buffer size in bytes.
    ///
    /// This controls the OS receive buffer. Larger values reduce packet loss
    /// under bursty traffic at the cost of memory.
    /// Default (if not set): OS-dependent.
    pub fn rcvbuf(&mut self, n: usize) -> &mut Self {
        self.config.rcvbuf = Some(n);
        self
    }

    /// Set SO_SNDBUF socket buffer size in bytes.
    ///
    /// This controls the OS send buffer. Larger values may improve throughput.
    /// Default (if not set): OS-dependent.
    pub fn sndbuf(&mut self, n: usize) -> &mut Self {
        self.config.sndbuf = Some(n);
        self
    }

    /// Set the maximum payload chunk size per `Transport::send_bytes` call.
    ///
    /// TCP is a bytestream; this is an upper bound on how much the crate
    /// accepts per single call. Larger values may reduce syscall overhead.
    /// Default: 64 KiB.
    pub fn pkt_size(&mut self, n: usize) -> &mut Self {
        self.config.pkt_size = Some(n);
        self
    }

    /// Set the connection timeout (caller-side only).
    ///
    /// Default: 10 seconds.
    pub fn connect_timeout(&mut self, t: Duration) -> &mut Self {
        self.config.connect_timeout = Some(t);
        self
    }

    /// Establish the TCP connection with accumulated config.
    ///
    /// # Errors
    ///
    /// Returns [`TcpError`] if:
    /// - The connection times out.
    /// - The connection is refused or unreachable.
    /// - TLS handshake fails (for `tcps://`).
    /// - The TLS feature is disabled but the URL used `tcps://`.
    pub fn build(self) -> Result<TcpTransport, TcpError> {
        TcpTransport::connect_with_config(&self.url, &self.config)
    }
}

/// Builder for [`TcpListener`].
///
/// Construct via [`TcpListenerBuilder::from_url`], optionally chain config methods,
/// then call [`build`](Self::build) to bind the listener socket.
///
/// # Example
///
/// ```no_run
/// # use tst_tcp::builder::TcpListenerBuilder;
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let listener = TcpListenerBuilder::from_url("tcp://0.0.0.0:5000?listen=1")?
///     .build()?;
/// # Ok(())
/// # }
/// ```
#[must_use]
#[derive(Debug, Clone)]
pub struct TcpListenerBuilder {
    url: TcpUrl,
    config: SocketConfig,
}

impl TcpListenerBuilder {
    /// Create a builder from a listener URL (`tcp://addr:port?listen=1` or
    /// `tcps://addr:port?listen=1&cert=...&key=...`).
    ///
    /// # Errors
    ///
    /// Returns [`TcpUrlError`] if the URL is malformed or does not have
    /// `?listen=1` (callers must use [`TcpTransportBuilder`] instead).
    pub fn from_url(url: &str) -> Result<Self, TcpUrlError> {
        let url = TcpUrl::parse(url)?;
        let mut config = SocketConfig::default();
        config.merge_from_url(&url);
        Ok(Self { url, config })
    }

    /// Set SO_RCVBUF for accepted connections.
    pub fn rcvbuf(&mut self, n: usize) -> &mut Self {
        self.config.rcvbuf = Some(n);
        self
    }

    /// Set SO_SNDBUF for accepted connections.
    pub fn sndbuf(&mut self, n: usize) -> &mut Self {
        self.config.sndbuf = Some(n);
        self
    }

    /// Set TCP_NODELAY for accepted connections.
    pub fn nodelay(&mut self, on: bool) -> &mut Self {
        self.config.nodelay = Some(on);
        self
    }

    /// Set the maximum payload chunk size for accepted connections.
    pub fn pkt_size(&mut self, n: usize) -> &mut Self {
        self.config.pkt_size = Some(n);
        self
    }

    /// Bind the listener socket.
    ///
    /// # Errors
    ///
    /// Returns [`TcpError`] if:
    /// - The port is already in use.
    /// - Insufficient permissions to bind.
    /// - TLS handshake setup fails (for `tcps://`).
    /// - The TLS feature is disabled but the URL used `tcps://`.
    pub fn build(self) -> Result<TcpListener, TcpError> {
        let mut listener = TcpListener::from_url(&self.format_url())?;
        // Apply any builder-set config overrides.
        *listener.config_mut() = self.config;
        Ok(listener)
    }

    fn format_url(&self) -> String {
        let scheme = if self.url.tls { "tcps" } else { "tcp" };
        // IPv6 literals must be bracketed in URLs (e.g. `[::1]`).
        let host = if self.url.host.contains(':') {
            format!("[{}]", self.url.host)
        } else {
            self.url.host.clone()
        };
        let mut url = format!("{}://{}:{}?listen=1", scheme, host, self.url.port);

        if let Some(cert) = &self.url.cert {
            url.push_str(&format!("&cert={}", cert));
        }
        if let Some(key) = &self.url.key {
            url.push_str(&format!("&key={}", key));
        }

        url
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caller_builder_chains() {
        let _b = TcpTransportBuilder::from_url("tcp://1.2.3.4:7001")
            .unwrap()
            .nodelay(true)
            .keepalive(Duration::from_secs(30))
            .rcvbuf(8 * 1024 * 1024)
            .pkt_size(65536);
    }

    #[test]
    fn listener_builder_chains() {
        let _b = TcpListenerBuilder::from_url("tcp://0.0.0.0:7001?listen=1")
            .unwrap()
            .nodelay(true)
            .rcvbuf(4 * 1024 * 1024)
            .sndbuf(4 * 1024 * 1024);
    }

    #[test]
    fn caller_url_parse_error_propagates() {
        let err = TcpTransportBuilder::from_url("invalid://host");
        assert!(err.is_err());
    }

    #[test]
    fn listener_url_parse_error_propagates() {
        let err = TcpListenerBuilder::from_url("invalid://host");
        assert!(err.is_err());
    }

    #[test]
    fn ipv6_listener_url_round_trips() {
        // format_url must bracket IPv6 literals so the re-parsed URL is valid.
        let b = TcpListenerBuilder::from_url("tcp://[::1]:7001?listen=1").unwrap();
        let formatted = b.format_url();
        let reparsed = crate::url::TcpUrl::parse(&formatted).expect("re-parse must succeed");
        assert_eq!(reparsed.host, "::1");
        assert!(reparsed.listen);
    }
}
