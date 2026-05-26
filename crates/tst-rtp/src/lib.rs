//! TS Transformer RTP transport — RTP-over-UDP per RFC 3550 carrying an
//! MPEG-TS bytestream per RFC 2250, plus an RTSP/1.0 + RTSP/2.0 client
//! (Phase 2) for negotiated unicast / multicast / TCP-interleaved
//! sessions. RTCP RR/SR populates `socket_stats().rtt_us` +
//! `packets_lost_send` end-to-end.
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
pub mod init;
pub mod packet;
pub mod rtcp;
pub mod rtsp;
pub mod sdp;
pub mod transport;
pub mod url;

// Phase 1 (unchanged).
pub use builder::{RtpRecvSocketBuilder, RtpSocketBuilder};
pub use cancel::RtpCancelHandle;
pub use clock::RtpClock;
pub use packet::{Parsed, RTP_HEADER_LEN, RTP_PT_MP2T, RTP_VERSION, RtpHeader, RtpParseError};
pub use transport::{ConnectError, RtpRecvTransport, RtpStats, RtpTransport};
pub use url::{DEFAULT_PKT_SIZE, RtpUrl, UrlError as RtpUrlError};

// Phase 2 — populated by subsequent tasks as items become real.
pub use error::RtspError;
pub use rtcp::ingest::{SrAnchor, compute_rtt_us, ingest_rr, ingest_sr, system_time_to_ntp_mid};
pub use rtcp::reporter::{RTCP_BASE_INTERVAL, RtcpReporterHandle};
pub use rtcp::stats::RtcpStats;
pub use rtcp::{ReceiverReport, ReportBlock, RtcpPacketType, SdesPacket, SenderReport};
pub use rtsp::auth::{AuthChallenge, DigestAlgorithm, DigestChallenge, DigestContext};
pub use rtsp::client::options_describe::OptionsResponse;
pub use rtsp::client::session::RtspSession;
pub use rtsp::client::transport_negotiation::{RtspTransportKind, TransportResponse};
pub use rtsp::client::{RtspCancelHandle, RtspClient};
pub use rtsp::interleaved::{Frame, InterleavedReader, InterleavedWriter, MAX_BINARY_FRAME_LEN};
pub use rtsp::message::{RtspMethod, RtspRequest, RtspResponse};
pub use sdp::pick::pick_mp2t;
pub use sdp::{Sdp, SdpMedia};
pub use url::{RtspScheme, RtspTransportPref, RtspUrl, RtspVersion};
