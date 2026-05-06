//! SRT-specific transport implementations.
//!
//! After the pipeline carve-out, only the SRT-specific Transport /
//! RecvTransport impls live here. The transport-agnostic shells
//! live in `tst_pipeline`; the trait definitions in `tst_core`.

pub mod srt_transport;

pub use srt_transport::SrtTransport;

// Re-export the transport-agnostic pipeline shells from tst-pipeline so
// existing `srt_core::pipeline::*` paths keep compiling.
pub use tst_pipeline::{
    ByteSink,
    DemuxReceiver, DemuxReceiverError, DemuxReceiverStats,
    ManagedReceiveTransport,
    ManagedTransport,
    MuxSender, MuxSenderError, MuxSenderStats,
    RawReceiver, RawReceiverStats,
    RawSender, RawSenderConfig, RawSenderStats,
    Receiver, ReceiverStats,
    Sender, SenderConfig, SenderError, SenderStats, TsFramingMode,
    BackoffStrategy, GapBuffer, OverflowPolicy, ReconnectPolicy,
};
pub use tst_pipeline::reconnect;
pub use tst_core::transport::{RecvTransport, Transport, TransportCancel, TransportError};
