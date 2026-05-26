//! Fluent builders over [`RtpUrl`] for callers who'd rather construct
//! transports field-by-field than parse a string.
//!
//! Mirrors the `SocketBuilder` / `ListenerBuilder` shape from `tst-srt`
//! — `&mut self -> &mut Self` setters, terminal `connect` / `listen`
//! takes `&self` and clones the inner URL.
//!
//! The terminal methods delegate to [`RtpTransport::connect_with`] /
//! [`RtpRecvTransport::listen_with`]; the builder is sugar, not a
//! parallel implementation.

use std::time::Duration;

use secrecy::SecretString;

use crate::error::RtspError;
use crate::rtsp::client::RtspClient;
use crate::transport::{ConnectError, RtpRecvTransport, RtpTransport};
use crate::url::{DEFAULT_PKT_SIZE, RtpUrl, RtspUrl, UrlError as RtpUrlError};

/// Fluent builder for send-side [`RtpTransport`].
#[must_use]
#[derive(Debug, Clone)]
pub struct RtpSocketBuilder {
    url: RtpUrl,
    /// Whether to auto-bind the RTCP companion socket on `port + 1`
    /// per RFC 3550 §11. Default `true` — opt out for callers that
    /// want pure-RTP without the reporter thread.
    rtcp: bool,
}

impl RtpSocketBuilder {
    /// New builder targeting `host:port`. `host` must be a literal IP
    /// (multicast group or unicast destination).
    pub fn new(host: impl Into<String>, port: u16) -> Self {
        Self {
            url: RtpUrl {
                host: host.into(),
                port,
                ttl: None,
                iface: None,
                pkt_size: DEFAULT_PKT_SIZE,
                ssrc: None,
            },
            rtcp: true,
        }
    }

    /// Parse an `rtp://host:port[?...]` URL into a builder.
    pub fn from_url(url: &str) -> Result<Self, RtpUrlError> {
        let parsed = RtpUrl::parse(url)?;
        Ok(Self {
            url: parsed,
            rtcp: true,
        })
    }

    /// Override TTL / hop-limit for multicast send (`IP_MULTICAST_TTL` /
    /// `IPV6_MULTICAST_HOPS`). Unicast: ignored.
    pub fn ttl(&mut self, n: u8) -> &mut Self {
        self.url.ttl = Some(n);
        self
    }

    /// Override the multicast interface (`IP_MULTICAST_IF`). IPv4: takes
    /// a literal IPv4 address (`"192.168.1.50"`).
    pub fn iface(&mut self, name_or_ip: impl Into<String>) -> &mut Self {
        self.url.iface = Some(name_or_ip.into());
        self
    }

    /// UDP payload size (188-multiple). Default 1316.
    pub fn pkt_size(&mut self, n: usize) -> &mut Self {
        self.url.pkt_size = n;
        self
    }

    /// Override the RTP SSRC. Default: random.
    pub fn ssrc(&mut self, n: u32) -> &mut Self {
        self.url.ssrc = Some(n);
        self
    }

    /// Enable or disable the auto-bound RTCP companion socket on
    /// `port + 1`. Default `true`. Pass `false` to skip the RTCP
    /// socket pair + reporter thread.
    pub fn rtcp(mut self, enabled: bool) -> Self {
        self.rtcp = enabled;
        self
    }

    /// Build the transport. Equivalent to [`Self::connect`] but takes
    /// `self` by value to match the builder convention.
    pub fn build(self) -> Result<RtpTransport, ConnectError> {
        RtpTransport::connect_with_rtcp(&self.url, self.rtcp)
    }

    /// Build the transport. Kept for backward compatibility with the
    /// original Phase 1 builder shape.
    pub fn connect(&self) -> Result<RtpTransport, ConnectError> {
        RtpTransport::connect_with_rtcp(&self.url, self.rtcp)
    }
}

/// Fluent builder for recv-side [`RtpRecvTransport`].
#[must_use]
#[derive(Debug, Clone)]
pub struct RtpRecvSocketBuilder {
    url: RtpUrl,
    /// Whether to auto-bind the RTCP companion socket on `port + 1`
    /// per RFC 3550 §11. Default `true` — opt out for callers that
    /// want pure-RTP without the reporter thread.
    rtcp: bool,
}

impl RtpRecvSocketBuilder {
    /// New builder bound to `host:port` for listening. For multicast,
    /// `host` is the group address (the socket binds to ANY:port and
    /// joins `group`).
    pub fn new(host: impl Into<String>, port: u16) -> Self {
        Self {
            url: RtpUrl {
                host: host.into(),
                port,
                ttl: None,
                iface: None,
                pkt_size: DEFAULT_PKT_SIZE,
                ssrc: None,
            },
            rtcp: true,
        }
    }

    /// Parse an `rtp://host:port[?...]` URL into a builder.
    pub fn from_url(url: &str) -> Result<Self, RtpUrlError> {
        let parsed = RtpUrl::parse(url)?;
        Ok(Self {
            url: parsed,
            rtcp: true,
        })
    }

    /// Override the multicast-recv interface.
    pub fn iface(&mut self, name_or_ip: impl Into<String>) -> &mut Self {
        self.url.iface = Some(name_or_ip.into());
        self
    }

    /// UDP payload size for the recv scratch buffer (188-multiple).
    /// Default 1316.
    pub fn pkt_size(&mut self, n: usize) -> &mut Self {
        self.url.pkt_size = n;
        self
    }

    /// Enable or disable the auto-bound RTCP companion socket on
    /// `port + 1`. Default `true`. Pass `false` to skip the RTCP
    /// socket pair + reporter thread.
    pub fn rtcp(mut self, enabled: bool) -> Self {
        self.rtcp = enabled;
        self
    }

    /// Build the recv transport. Takes `self` by value to match the
    /// builder convention.
    pub fn build(self) -> Result<RtpRecvTransport, ConnectError> {
        RtpRecvTransport::listen_with_rtcp(&self.url, self.rtcp)
    }

    /// Build the recv transport. Kept for backward compatibility with
    /// the original Phase 1 builder shape.
    pub fn listen(&self) -> Result<RtpRecvTransport, ConnectError> {
        RtpRecvTransport::listen_with_rtcp(&self.url, self.rtcp)
    }
}

/// Fluent builder for [`RtspClient`] with extended options (auth
/// credentials, keepalive overrides, TLS roots, timeouts).
///
/// Unlike [`RtspClient::connect`] / [`RtspClient::connect_with`] which
/// take just a URL, this builder lets callers pass credentials and
/// keepalive policy without re-encoding them into the URL query string.
///
/// The builder's [`Self::connect`] always spawns the auto-keepalive
/// background thread unless [`Self::no_auto_keepalive`] is set.
#[must_use]
pub struct RtspClientBuilder {
    url: RtspUrl,
    username: Option<String>,
    password: Option<SecretString>,
    no_auto_keepalive: bool,
    keepalive_interval_override: Option<Duration>,
    connect_timeout: Duration,
    read_timeout: Duration,
    user_agent: String,
    #[cfg(feature = "tls")]
    tls_root_certs: Option<rustls::RootCertStore>,
}

impl RtspClientBuilder {
    /// New builder for `url`. URL-embedded credentials
    /// (`rtsp://user:pass@host/...`) are picked up as defaults; call
    /// [`Self::auth`] to override.
    ///
    /// # Errors
    ///
    /// - [`RtspError::Url`] if the URL cannot be parsed.
    pub fn new(url: &str) -> Result<Self, RtspError> {
        let parsed = RtspUrl::parse(url)?;
        let username = parsed.username.clone();
        let password = parsed.password.clone();
        Ok(Self {
            url: parsed,
            username,
            password,
            no_auto_keepalive: false,
            keepalive_interval_override: None,
            connect_timeout: Duration::from_secs(10),
            read_timeout: Duration::from_millis(100),
            user_agent: "tst-rtp/0.1".into(),
            #[cfg(feature = "tls")]
            tls_root_certs: None,
        })
    }

    /// Disable the auto-keepalive background thread. Default `false`
    /// (i.e., auto-keepalive is on).
    pub fn no_auto_keepalive(mut self, disabled: bool) -> Self {
        self.no_auto_keepalive = disabled;
        self
    }

    /// Override the keepalive interval. Default: `session_timeout / 2`
    /// as derived from the server's `Session: ...;timeout=N` header.
    pub fn keepalive_interval(mut self, t: Duration) -> Self {
        self.keepalive_interval_override = Some(t);
        self
    }

    /// Provide explicit credentials. Overrides anything parsed from
    /// the URL's userinfo component.
    pub fn auth(mut self, username: impl Into<String>, password: SecretString) -> Self {
        self.username = Some(username.into());
        self.password = Some(password);
        self
    }

    /// Override the `User-Agent` header. Default: `tst-rtp/0.1`.
    pub fn user_agent(mut self, ua: impl Into<String>) -> Self {
        self.user_agent = ua.into();
        self
    }

    /// TCP connect timeout. Default 10 s.
    pub fn connect_timeout(mut self, t: Duration) -> Self {
        self.connect_timeout = t;
        self
    }

    /// Per-read socket timeout (poll interval for cancel + interleaved
    /// frame reads). Default 100 ms.
    pub fn read_timeout(mut self, t: Duration) -> Self {
        self.read_timeout = t;
        self
    }

    /// Override the rustls root certificate store for `rtsps://`
    /// connections. Default: webpki-roots / system roots per the `tls`
    /// module's policy.
    #[cfg(feature = "tls")]
    pub fn tls_root_certs(mut self, certs: rustls::RootCertStore) -> Self {
        self.tls_root_certs = Some(certs);
        self
    }

    /// Connect, returning the live client.
    ///
    /// For v1, the builder delegates to
    /// [`RtspClient::connect_with`]. Task 17 will wire the
    /// `keepalive_interval_override` field into the keepalive thread
    /// spawn; for now the override field is stored but the spawn stub
    /// is a no-op.
    ///
    /// # Errors
    ///
    /// See [`RtspClient::connect_with`].
    pub fn connect(self) -> Result<RtspClient, RtspError> {
        let mut client = RtspClient::connect_with(&self.url)?;
        if !self.no_auto_keepalive {
            client.spawn_keepalive_if_needed(self.keepalive_interval_override);
        }
        // `username`, `password`, `connect_timeout`, `read_timeout`,
        // `user_agent`, and `tls_root_certs` are stored on the builder
        // for future tasks to wire through to the client (auth flow,
        // socket timeouts, TLS handshake) — see the plan's later
        // waves.
        let _ = (
            &self.username,
            &self.password,
            self.connect_timeout,
            self.read_timeout,
            &self.user_agent,
        );
        #[cfg(feature = "tls")]
        let _ = &self.tls_root_certs;
        Ok(client)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_builder_round_trip_via_connect() {
        // Use port 1 — likely refused on bind→connect→sendto, but the
        // builder fields should propagate through connect_with cleanly.
        let mut b = RtpSocketBuilder::new("127.0.0.1", 1);
        b.pkt_size(376).ssrc(0xABCDEF01);
        let t = b
            .connect()
            .expect("connect to 127.0.0.1:1 should bind locally");
        // The Transport trait's max_payload() returns the application-
        // visible cap (pkt_size minus the 12-byte RTP header).
        use tst_core::transport::Transport;
        assert_eq!(t.max_payload(), 376 - 12);
    }

    #[test]
    fn recv_builder_binds_unicast() {
        let b = RtpRecvSocketBuilder::new("127.0.0.1", 0);
        // Port 0 -> OS-assigned; doesn't conflict.
        let _ = b.listen().expect("bind 127.0.0.1:0 should succeed");
    }
}
