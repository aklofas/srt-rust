//! RTCP RR / SR / SDES packet encoding, decoding, reporting, and ingest.
//!
//! Folded into Phase 2 to populate
//! [`tst_core::transport::SocketStats::rtt_us`] (via SR LSR/DLSR) and
//! [`tst_core::transport::SocketStats::packets_lost_send`] (via RR
//! fraction-lost).
//!
//! Module layout filled in by Tasks 7-11.

pub mod ingest;
pub mod reporter;
pub mod stats;
