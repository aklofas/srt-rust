//! `RtspSession` is the post-SETUP state held by `RtspClient::setup`'s
//! return value. Calling `into_recv_transport` converts it into a
//! `RtpRecvTransport` ready for `DemuxReceiver::new`.
//!
//! Two internal variants:
//!
//! - UDP-backed: holds the already-bound `UdpSocket` pair from SETUP.
//! - TCP-interleaved-backed: holds the consumer side of the mpsc
//!   channel fed by `RtspClient`'s pump thread (spawned in SETUP via
//!   the crate-private `RtspClient::activate_interleaved_pump`).

use std::net::{SocketAddr, UdpSocket};
use std::sync::mpsc;

use bytes::Bytes;

use crate::rtsp::client::transport_negotiation::{RtspTransportKind, TransportResponse};
use crate::transport::RtpRecvTransport;

/// State held between SETUP and PLAY / TEARDOWN.
//
// `dead_code` allowed on legacy fields that the SETUP code path may not
// touch on all flow combinations (peer_addr for TcpInterleaved, etc.).
#[allow(dead_code)]
pub struct RtspSession {
    pub(crate) session_id: String,
    pub(crate) transport: TransportResponse,
    pub(crate) kind: RtspTransportKind,
    pub(crate) udp_sockets: Option<(UdpSocket, UdpSocket)>,
    pub(crate) peer_addr: Option<SocketAddr>,
    /// For TcpInterleaved: consumer side of the pump's data channel.
    /// `None` for UDP sessions.
    pub(crate) data_rx: Option<mpsc::Receiver<Bytes>>,
    /// For TcpInterleaved: consumer side of the pump's RTCP channel.
    /// `None` for UDP sessions (UDP carries RTCP on its own dedicated
    /// socket pair). T27 (Phase 4 Stage 3) wires the pump's RTCP demux
    /// onto this `mpsc::Receiver<Bytes>`; T28 will consume it inside
    /// [`Self::into_recv_transport`] and feed each frame to the
    /// `RtcpReporterHandle` for receiver-side stats. Until T28 lands,
    /// the receiver is held here and dropped at session drop — which
    /// closes the pump's `mpsc::Sender` side and lets the pump observe
    /// `SendError` once the consumer goes away.
    pub(crate) rtcp_rx: Option<mpsc::Receiver<Bytes>>,
}

impl RtspSession {
    pub(crate) fn new_udp(
        sid: String,
        rtp: UdpSocket,
        rtcp: UdpSocket,
        transport: TransportResponse,
        peer: SocketAddr,
    ) -> Self {
        Self {
            session_id: sid,
            kind: RtspTransportKind::Udp,
            transport,
            udp_sockets: Some((rtp, rtcp)),
            peer_addr: Some(peer),
            data_rx: None,
            rtcp_rx: None,
        }
    }

    /// Construct a TCP-interleaved session, carrying both the pump's
    /// data `mpsc::Receiver<Bytes>` (so [`Self::into_recv_transport`]
    /// can hand it to [`RtpRecvTransport::from_mpsc_placeholder`] for
    /// the consumer side) and the pump's RTCP `mpsc::Receiver<Bytes>`
    /// (T28 will route into `RtcpReporterHandle`).
    pub(crate) fn new_interleaved_with_data_rx(
        sid: String,
        transport: TransportResponse,
        data_rx: mpsc::Receiver<Bytes>,
        rtcp_rx: mpsc::Receiver<Bytes>,
    ) -> Self {
        Self {
            session_id: sid,
            kind: RtspTransportKind::TcpInterleaved,
            transport,
            udp_sockets: None,
            peer_addr: None,
            data_rx: Some(data_rx),
            rtcp_rx: Some(rtcp_rx),
        }
    }

    /// Session ID returned by the server at SETUP. Echoed back in PLAY /
    /// PAUSE / TEARDOWN under the `Session:` header.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Which transport flavor the server picked at SETUP.
    pub fn transport_kind(&self) -> RtspTransportKind {
        self.kind
    }

    /// Peer's RTCP endpoint (`peer.ip():server_rtcp_port`) for UDP
    /// sessions, `None` for TCP-interleaved (the RTCP carrier is the
    /// control TCP itself in that case).
    pub fn rtcp_endpoint(&self) -> Option<SocketAddr> {
        let peer = self.peer_addr?;
        let port = self.transport.server_port.map(|(_, hi)| hi)?;
        Some(SocketAddr::new(peer.ip(), port))
    }

    /// Consume the session and return an `RtpRecvTransport` ready for
    /// `DemuxReceiver`.
    ///
    /// For UDP: wraps the SETUP-allocated UDP socket pair into a
    /// pre-built `RtpRecvTransport` (avoiding a second `listen()` call).
    ///
    /// For TCP-interleaved: returns an `RtpRecvTransport` fed by the
    /// `mpsc::Receiver<Bytes>` populated by `RtspClient`'s
    /// interleaved-pump thread (spawned at SETUP time by the
    /// crate-private `RtspClient::activate_interleaved_pump`). The
    /// pump's RTCP `mpsc::Receiver<Bytes>` is also handed in and drives
    /// the new `rtsp-rtcp-ingest` thread inside the transport — that
    /// thread populates `socket_stats().rtt_us` +
    /// `socket_stats().packets_lost_send` from RR + SR frames arriving
    /// on RFC 7826 §14 channel 1.
    pub fn into_recv_transport(mut self) -> RtpRecvTransport {
        match self.kind {
            RtspTransportKind::Udp => {
                let (rtp, _rtcp) = self.udp_sockets.expect("UDP session has sockets");
                RtpRecvTransport::from_udp_socket(rtp).expect("from_udp_socket")
            }
            RtspTransportKind::TcpInterleaved => {
                let data_rx = self
                    .data_rx
                    .take()
                    .expect("TcpInterleaved session has a pump data_rx");
                let rtcp_rx = self
                    .rtcp_rx
                    .take()
                    .expect("TcpInterleaved session has a pump rtcp_rx");
                RtpRecvTransport::from_mpsc_with_rtcp(data_rx, rtcp_rx)
            }
        }
    }
}
