//! RTCP-derived counters, exposed alongside [`tst_core::transport::SocketStats`]
//! via `RtpRecvTransport::rtcp_stats` / `RtpTransport::rtcp_stats`.

use std::time::SystemTime;

/// RTCP-specific protocol counters.
#[derive(Debug, Clone, Default)]
pub struct RtcpStats {
    pub rr_packets_received: u64,
    pub sr_packets_received: u64,
    pub rr_packets_sent: u64,
    pub sr_packets_sent: u64,
    /// Last RR ingest timestamp (system time of `recv`).
    pub last_rr_ts: Option<SystemTime>,
    /// Last SR ingest timestamp.
    pub last_sr_ts: Option<SystemTime>,
    /// Interarrival jitter in microseconds, from the most recent RR
    /// from peer.
    pub interarrival_jitter_us: u32,
    /// Q8 fixed-point fraction-lost from the most recent RR.
    pub fraction_lost_q8: u8,
    /// Number of RTCP packets dropped due to parse errors.
    pub rr_parse_errors: u64,
    pub sr_parse_errors: u64,
}
