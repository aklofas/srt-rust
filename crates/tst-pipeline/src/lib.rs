//! TS Transformer pipeline shells.
//!
//! Provides the standard sender and receiver shells over the
//! transport traits defined in [`tst_core`]. Concrete transport
//! impls live in dedicated crates (`srt-core`, future `tst-srt`,
//! `tst-udp`, `tst-rtp`, `tst-tcp`, `tst-rtsp`).

pub mod mux_sender;
pub mod sender;
pub mod raw_sender;
pub mod demux_receiver;
pub mod receiver;
pub mod raw_receiver;
pub mod managed_receive;
pub mod reconnect;

// Top-level re-exports of the most common types.
pub use mux_sender::{MuxSender, MuxSenderError, MuxSenderStats};
pub use sender::{Sender, SenderError, SenderStats, SenderConfig, TsFramingMode};
pub use raw_sender::{RawSender, RawSenderConfig, RawSenderStats};
pub use demux_receiver::{ByteSink, DemuxReceiver, DemuxReceiverError, DemuxReceiverStats};
pub use receiver::{Receiver, ReceiverStats};
pub use raw_receiver::{RawReceiver, RawReceiverStats};
pub use managed_receive::ManagedReceiveTransport;
pub use reconnect::{ManagedTransport, ReconnectPolicy, BackoffStrategy, GapBuffer, OverflowPolicy};

// Re-export the core trait types for caller convenience.
pub use tst_core::transport::{Transport, TransportError, TransportCancel, RecvTransport};
