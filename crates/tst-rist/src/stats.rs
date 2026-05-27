//! [`RistStats`] — sender + receiver stats projections.

use tst_core::transport::SocketStats;

/// Cumulative stats for a single RIST transport handle.
///
/// librist exposes much richer counters via its `rist_stats` callback (sent /
/// received / retransmitted / dropped / RTT / bandwidth) — those are surfaced
/// here as a flat struct after periodic polling.
#[derive(Debug, Clone, Copy, Default)]
#[non_exhaustive]
pub struct RistStats {
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub packets_sent: u64,
    pub packets_received: u64,
    pub packets_retransmitted: u64,
    pub packets_dropped: u64,
    /// Smoothed bandwidth, kbps.
    pub bandwidth_kbps: u32,
    /// Smoothed RTT, microseconds.
    pub rtt_us: u32,
}

impl RistStats {
    /// Project to the workspace-uniform [`SocketStats`].
    ///
    /// `SocketStats` is `#[non_exhaustive]`. Both the struct-expression-with-
    /// `..Default::default()` form and the bare default-and-field-assign form
    /// work from this crate (we're outside `tst-core`'s defining scope, but
    /// `..Default::default()` is still permitted at the boundary). The
    /// default-and-assign form is more concise here.
    pub fn to_socket_stats(&self) -> SocketStats {
        let mut s = SocketStats::default();
        s.bytes_sent = self.bytes_sent;
        s.packets_sent = self.packets_sent;
        s.bytes_received = self.bytes_received;
        s.packets_received = self.packets_received;
        s.packets_retransmitted = self.packets_retransmitted;
        s.rtt_us = self.rtt_us;
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection() {
        let r = RistStats {
            bytes_sent: 1000,
            packets_sent: 5,
            packets_retransmitted: 1,
            rtt_us: 12_345,
            ..RistStats::default()
        };
        let s = r.to_socket_stats();
        assert_eq!(s.bytes_sent, 1000);
        assert_eq!(s.packets_sent, 5);
        assert_eq!(s.packets_retransmitted, 1);
        assert_eq!(s.rtt_us, 12_345);
    }

    #[test]
    fn default_is_all_zeros() {
        let r = RistStats::default();
        assert_eq!(r.bytes_sent, 0);
        assert_eq!(r.packets_dropped, 0);
        assert_eq!(r.rtt_us, 0);
    }
}
