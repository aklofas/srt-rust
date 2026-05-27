//! Outbound-only, segment-aware sinks (HLS, future MPEG-DASH, ...).
//!
//! Sits alongside [`crate::transport::Transport`] and
//! [`crate::transport::RecvTransport`] as a third trait family.
//! First implementation: `HlsPublisher` in the `tst-tcp` crate.
