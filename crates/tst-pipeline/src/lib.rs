//! TS Transformer pipeline shells.
//!
//! Provides the standard sender and receiver shells over the
//! transport traits defined in [`tst_core`]. Concrete transport
//! impls live in dedicated crates (`srt-core`, future `tst-srt`,
//! `tst-udp`, `tst-rtp`, `tst-tcp`, `tst-rtsp`).

pub mod demux_receiver;
pub mod managed_receive;
pub mod mux_sender;
pub mod raw_receiver;
pub mod raw_sender;
pub mod receiver;
pub mod reconnect;
pub mod sender;

// Top-level re-exports of the most common types.
pub use demux_receiver::{ByteSink, DemuxReceiver, DemuxReceiverError, DemuxReceiverStats};
pub use managed_receive::ManagedReceiveTransport;
pub use mux_sender::{MuxSender, MuxSenderError, MuxSenderStats};
pub use raw_receiver::{RawReceiver, RawReceiverStats};
pub use raw_sender::{RawSender, RawSenderConfig, RawSenderStats};
pub use receiver::{Receiver, ReceiverStats};
pub use reconnect::{
    BackoffStrategy, GapBuffer, ManagedTransport, OverflowPolicy, ReconnectPolicy,
};
pub use sender::{Sender, SenderConfig, SenderError, SenderStats, TsFramingMode};

// Re-export the core trait types for caller convenience.
pub use tst_core::transport::{RecvTransport, Transport, TransportCancel, TransportError};
