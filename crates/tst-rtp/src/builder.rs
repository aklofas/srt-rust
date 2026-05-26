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

use crate::transport::{ConnectError, RtpRecvTransport, RtpTransport};
use crate::url::{DEFAULT_PKT_SIZE, RtpUrl, UrlError as RtpUrlError};

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
