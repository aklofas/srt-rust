//! UDP-first SETUP attempt with auto-fallback to TCP-interleaved on 461.
//!
//! Trigger for fallback: server response code 461 Unsupported Transport
//! (RFC 7826 §17.4.6). Any other 4xx/5xx surfaces as
//! `RtspError::Protocol { code, reason }`.
//!
//! `?transport=udp` in the URL skips fallback; `?transport=tcp` skips
//! the UDP attempt entirely.

use std::net::UdpSocket;

use crate::error::RtspError;
use crate::url::RtspTransportPref;

/// What kind of transport was negotiated by SETUP.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RtspTransportKind {
    /// Plain UDP unicast: RTP on one even port, RTCP on the next odd port.
    Udp,
    /// TCP-interleaved framing per RFC 7826 §14 over the RTSP control
    /// connection itself.
    TcpInterleaved,
}

/// Parsed `Transport:` response header.
#[derive(Debug, Clone)]
pub struct TransportResponse {
    /// Which transport the server actually accepted.
    pub kind: RtspTransportKind,
    /// For UDP: server's RTP+RTCP port range from `server_port=lo-hi`.
    pub server_port: Option<(u16, u16)>,
    /// For TCP-interleaved: channel range from `interleaved=lo-hi`.
    pub interleaved: Option<(u8, u8)>,
    /// `ssrc=` parameter from the Transport header, if present.
    pub ssrc: Option<u32>,
}

/// Parse the server's `Transport:` response header.
///
/// Recognized keys: `server_port=`, `interleaved=`, `ssrc=`. The kind is
/// inferred from the presence of `RTP/AVP/TCP` or `interleaved=`.
pub fn parse_transport_response(header_value: &str) -> Result<TransportResponse, RtspError> {
    let lower = header_value.to_ascii_lowercase();
    let kind = if lower.contains("rtp/avp/tcp") || lower.contains("interleaved=") {
        RtspTransportKind::TcpInterleaved
    } else {
        RtspTransportKind::Udp
    };
    let mut server_port = None;
    let mut interleaved = None;
    let mut ssrc = None;
    for part in header_value.split(';') {
        let part = part.trim();
        if let Some(v) = part.strip_prefix("server_port=") {
            let mut it = v.split('-');
            let lo = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
            let hi = it.next().and_then(|s| s.parse().ok()).unwrap_or(lo + 1);
            server_port = Some((lo, hi));
        }
        if let Some(v) = part.strip_prefix("interleaved=") {
            let mut it = v.split('-');
            let lo = it.next().and_then(|s| s.parse::<u8>().ok()).unwrap_or(0);
            let hi = it
                .next()
                .and_then(|s| s.parse::<u8>().ok())
                .unwrap_or(lo + 1);
            interleaved = Some((lo, hi));
        }
        if let Some(v) = part.strip_prefix("ssrc=") {
            ssrc = u32::from_str_radix(v.trim(), 16).ok();
        }
    }
    Ok(TransportResponse {
        kind,
        server_port,
        interleaved,
        ssrc,
    })
}

/// Bind a UDP RTP+RTCP port pair on `0.0.0.0`.
///
/// Returns `(rtp_socket, rtcp_socket, rtp_port)`. Tries up to 8 candidate
/// even-port pairs starting from `start` — the first pair where both ports
/// bind cleanly is returned.
///
/// # Errors
///
/// [`RtspError::Io`] with `AddrInUse` if all 8 candidate pairs fail.
pub fn bind_udp_pair(start: u16) -> Result<(UdpSocket, UdpSocket, u16), RtspError> {
    for attempt in 0..8 {
        let port = start + (attempt * 2);
        if let Ok(rtp) = UdpSocket::bind(("0.0.0.0", port)) {
            if let Ok(rtcp) = UdpSocket::bind(("0.0.0.0", port + 1)) {
                return Ok((rtp, rtcp, port));
            }
        }
    }
    Err(RtspError::Io(std::io::ErrorKind::AddrInUse))
}

/// Build the outgoing `Transport:` header value per the URL's
/// `transport_preference`.
///
/// `PreferUdp` and `ForceUdp` both emit a UDP request; the difference is
/// the fallback policy in [`super::setup`].
pub fn build_transport_request(pref: RtspTransportPref, rtp_port: u16) -> String {
    match pref {
        RtspTransportPref::PreferUdp | RtspTransportPref::ForceUdp => {
            format!("RTP/AVP;unicast;client_port={}-{}", rtp_port, rtp_port + 1)
        }
        RtspTransportPref::ForceTcp => "RTP/AVP/TCP;unicast;interleaved=0-1".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_udp_transport_response() {
        let h = "RTP/AVP;unicast;client_port=5004-5005;server_port=6970-6971;ssrc=0DEADBEEF";
        let t = parse_transport_response(h).unwrap();
        assert_eq!(t.kind, RtspTransportKind::Udp);
        assert_eq!(t.server_port, Some((6970, 6971)));
    }

    #[test]
    fn parse_interleaved_transport_response() {
        let h = "RTP/AVP/TCP;unicast;interleaved=0-1";
        let t = parse_transport_response(h).unwrap();
        assert_eq!(t.kind, RtspTransportKind::TcpInterleaved);
        assert_eq!(t.interleaved, Some((0, 1)));
    }

    #[test]
    fn build_udp_request_includes_port_pair() {
        let h = build_transport_request(RtspTransportPref::PreferUdp, 8000);
        assert!(h.contains("client_port=8000-8001"));
    }

    #[test]
    fn build_tcp_request_uses_interleaved_0_1() {
        let h = build_transport_request(RtspTransportPref::ForceTcp, 0);
        assert!(h.contains("interleaved=0-1"));
    }
}
