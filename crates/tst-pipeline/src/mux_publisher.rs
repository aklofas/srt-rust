//! [`MuxPublisher`] — pipeline shell that owns a [`Muxer`] and pushes its
//! output to a [`Publisher`]. Mirrors [`MuxSender`] but for outbound-only,
//! segment-aware sinks.
//!
//! [`Muxer`]: tst_core::mpegts::mux::Muxer
//! [`Publisher`]: tst_core::publisher::Publisher
//! [`MuxSender`]: crate::MuxSender
