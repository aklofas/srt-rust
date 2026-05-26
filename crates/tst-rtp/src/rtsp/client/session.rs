//! `RtspSession` is the post-SETUP state held by `RtspClient::setup`'s
//! return value. Calling `into_recv_transport` converts it into a
//! `RtpRecvTransport` ready for `DemuxReceiver::new`.
//!
//! Two internal variants: UDP-backed (uses an already-bound
//! `UdpSocket` pair) and TCP-interleaved-backed (will be wired up to
//! the InterleavedReader queue in Wave D Task 17 via the RtspClient's
//! background thread).

use std::net::{SocketAddr, UdpSocket};

use crate::rtsp::client::transport_negotiation::{RtspTransportKind, TransportResponse};
use crate::transport::RtpRecvTransport;

/// State held between SETUP and PLAY / TEARDOWN.
//
// `dead_code` allowed: `new_udp` / `new_interleaved` are constructed
// by Task 13's SETUP code, which lands in parallel with this task.
#[allow(dead_code)]
pub struct RtspSession {
    pub(crate) session_id: String,
    pub(crate) transport: TransportResponse,
    pub(crate) kind: RtspTransportKind,
    pub(crate) udp_sockets: Option<(UdpSocket, UdpSocket)>,
    pub(crate) peer_addr: Option<SocketAddr>,
}

// `dead_code` allowed: the `new_udp` / `new_interleaved` constructors
// are called by Task 13's SETUP code, which lands in parallel with this
// task. Once Wave C merges, those callers light up.
#[allow(dead_code)]
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
        }
    }

    pub(crate) fn new_interleaved(sid: String, transport: TransportResponse) -> Self {
        Self {
            session_id: sid,
            kind: RtspTransportKind::TcpInterleaved,
            transport,
            udp_sockets: None,
            peer_addr: None,
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
    /// For TCP-interleaved: returns an `RtpRecvTransport` whose
    /// internal source is the `mpsc::Receiver<Bytes>` fed by the
    /// `RtspClient`'s `InterleavedReader` background thread. The bridge
    /// from InterleavedReader to this transport's queue is finalized in
    /// Wave D Task 17 (Keepalive + InterleavedReader wiring).
    pub fn into_recv_transport(self) -> RtpRecvTransport {
        match self.kind {
            RtspTransportKind::Udp => {
                let (rtp, _rtcp) = self.udp_sockets.expect("UDP session has sockets");
                RtpRecvTransport::from_udp_socket(rtp).expect("from_udp_socket")
            }
            RtspTransportKind::TcpInterleaved => {
                // Task 17 wires the InterleavedReader bridge so the
                // resulting transport feeds from an mpsc::Receiver<Bytes>
                // populated by the RtspClient's background reader. The
                // full producer-side spawn lands in a later wave; here
                // we hand the consumer a never-fed channel so the
                // transport compiles + behaves like an idle source
                // (recv_timeout loops on cancel-flag polls).
                let (_tx, rx) = std::sync::mpsc::channel::<bytes::Bytes>();
                RtpRecvTransport::from_mpsc_placeholder(rx)
            }
        }
    }
}
