//! TS Transformer RTP transport — RTP-over-UDP per RFC 3550 carrying an
//! MPEG-TS bytestream per RFC 2250, plus an RTSP/1.0 client (Phase 2) and
//! server (Phase 3) for negotiated unicast / multicast / TCP-interleaved
//! sessions. RTSP/2.0 (RFC 7826) requests are handled on the RFC 7826-
//! compatible subset shared with RTSP/1.0; the server does not implement
//! RTSP/2.0-only features (e.g. pipelining, REDIRECT, per-request timeouts).
//! As of Phase 4 Stage 3, RTCP RR/SR ingest is
//! wired on the TCP-interleaved (RFC 7826 §14) client path: peer RR
//! populates `socket_stats().packets_lost_send` from the cumulative-lost
//! field and `socket_stats().rtt_us` from the RR-after-SR calculation
//! (RFC 3550 §6.4.1). UDP-side RTCP ingest is deferred.
//!
//! This crate provides the RTP-specific concrete types. The
//! [`Transport`](tst_core::transport::Transport) /
//! [`RecvTransport`](tst_core::transport::RecvTransport) traits themselves
//! live in [`tst_core`]; the transport-agnostic Sender/Receiver shells
//! live in `tst_pipeline`.

#![warn(rustdoc::broken_intra_doc_links)]

pub mod builder;
pub mod cancel;
pub mod clock;
pub mod error;
pub mod h264;
pub mod init;
pub mod packet;
pub mod rtcp;
pub mod rtsp;
pub mod sdp;
pub mod transport;
pub mod url;

// RTP transport.
pub use builder::{RtpRecvSocketBuilder, RtpSocketBuilder};
pub use cancel::RtpCancelHandle;
pub use clock::RtpClock;
pub use packet::{Parsed, RTP_HEADER_LEN, RTP_PT_MP2T, RTP_VERSION, RtpHeader, RtpParseError};
pub use transport::{ConnectError, RtpRecvTransport, RtpStats, RtpTransport};
pub use url::{DEFAULT_PKT_SIZE, RtpUrl, UrlError as RtpUrlError};

// RTSP client.
pub use builder::RtspClientBuilder;
pub use error::RtspError;
pub use rtcp::ingest::{SrAnchor, compute_rtt_us, ingest_rr, ingest_sr, system_time_to_ntp_mid};
pub use rtcp::reporter::{RTCP_BASE_INTERVAL, RtcpReporterHandle};
pub use rtcp::stats::RtcpStats;
pub use rtcp::{ReceiverReport, ReportBlock, RtcpError, RtcpPacketType, SdesPacket, SenderReport};
pub use rtsp::auth::{AuthChallenge, DigestAlgorithm, DigestChallenge, DigestContext};
pub use rtsp::client::options_describe::OptionsResponse;
pub use rtsp::client::play::{RtpInfo, parse_rtp_info};
pub use rtsp::client::session::RtspSession;
pub use rtsp::client::transport_negotiation::{RtspTransportKind, TransportResponse};
pub use rtsp::client::{RtspCancelHandle, RtspClient};
pub use rtsp::interleaved::{Frame, InterleavedReader, InterleavedWriter, MAX_BINARY_FRAME_LEN};
pub use rtsp::message::{RtspMethod, RtspRequest, RtspResponse};
pub use sdp::pick::pick_mp2t;
pub use sdp::{Sdp, SdpMedia};
pub use url::{RtspScheme, RtspTransportPref, RtspUrl, RtspVersion};

// H.264 RTP payload format (RFC 6184).
pub use h264::{
    H264Au, H264Depacketizer, H264DepayConfig, H264DepayStats, H264FmtpParams,
    ParameterSetInjection, parse_rtpmap_h264,
};

// RTSP server.
pub use builder::RtspServerBuilder;
pub use cancel::RtspServerCancelHandle;
pub use error::{MountError, RtspServerError};
pub use rtsp::server::mount::{MountHandle, MountKind, MountStats};
pub use rtsp::server::{RtspServer, ServerStats};
pub use url::MulticastGroup;
