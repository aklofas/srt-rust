// crates/srt-core/src/pipeline/mod.rs
//! Composition layer wiring `mpegts::mux::Muxer`, `klv::*`, and `srt::Socket`
//! into ergonomic sender and receiver shells for the canonical
//! platform-to-ground-station send path and ground-station receive path.
//!
//! **Send side:** three sender shells (`MuxSender`, `Sender`, `RawSender`) are
//! generic over a `Transport`; the `ManagedTransport` decorator (in
//! `pipeline::reconnect`) wraps any of them with reconnect + gap buffering.
//!
//! **Receive side:** receive shells (`RawReceiver`, `Receiver`, `DemuxReceiver`)
//! are generic over a `RecvTransport`. `SrtTransport` implements both
//! `Transport` and `RecvTransport`.

pub mod demux_receiver;
pub mod managed_receive;
pub mod mux_sender;
pub mod raw_receiver;
pub mod raw_sender;
pub mod receiver;
pub mod reconnect;
pub mod recv_transport;
pub mod sender;
pub mod srt_transport;
pub mod transport;

pub use demux_receiver::{ByteSink, DemuxReceiver, DemuxReceiverError, DemuxReceiverStats};
pub use managed_receive::ManagedReceiveTransport;
pub use mux_sender::{MuxSender, MuxSenderError, MuxSenderStats};
pub use raw_receiver::{RawReceiver, RawReceiverStats};
pub use raw_sender::{RawSender, RawSenderConfig, RawSenderStats};
pub use receiver::{Receiver, ReceiverStats};
pub use reconnect::{BackoffStrategy, ManagedTransport, OverflowPolicy, ReconnectPolicy};
pub use recv_transport::RecvTransport;
pub use sender::{Sender, SenderConfig, SenderError, SenderStats, TsFramingMode};
pub use srt_transport::SrtTransport;
pub use transport::{Transport, TransportCancel, TransportError};
