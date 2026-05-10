//! TS Transformer pipeline shells.
//!
//! Provides the standard sender and receiver shells over the
//! transport traits defined in [`tst_core`]. Concrete transport
//! impls live in dedicated crates (`tst-srt` today; future
//! `tst-udp`, `tst-rtp`, `tst-tcp`, `tst-rtsp`).
//!
//! ## Quick start — push pre-muxed TS bytes through any [`Transport`]
//!
//! ```
//! use tst_pipeline::{Sender, SenderConfig};
//! use tst_core::transport::{Transport, TransportError};
//!
//! // Trivial in-memory sink so the example needs no network. Real
//! // consumers plug in `tst_srt::SrtTransport` (or any other
//! // `Transport` impl) here.
//! struct Sink(Vec<u8>);
//! impl Transport for Sink {
//!     fn send_bytes(&mut self, b: &[u8]) -> Result<(), TransportError> {
//!         self.0.extend_from_slice(b);
//!         Ok(())
//!     }
//!     fn max_payload(&self) -> usize { 1316 }
//!     fn close(&mut self) {}
//!     fn is_alive(&self) -> bool { true }
//! }
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let mut sender = Sender::new(Sink(Vec::new()), SenderConfig::default());
//!
//! // One pre-muxed TS packet (188 bytes, sync byte 0x47 first).
//! let mut pkt = vec![0x47u8];
//! pkt.extend(vec![0u8; 187]);
//! sender.send_ts(&pkt)?;
//! sender.flush()?;
//! # Ok(())
//! # }
//! ```

#![warn(rustdoc::broken_intra_doc_links)]

pub mod demux_receiver;
pub mod dyn_aliases;
pub mod managed_receive;
pub mod mux_sender;
pub mod pairing;
pub mod raw_receiver;
pub mod raw_sender;
pub mod receiver;
pub mod reconnect;
pub mod sender;

// Top-level re-exports of the most common types.
pub use demux_receiver::{ByteSink, DemuxReceiver, DemuxReceiverError, DemuxReceiverStats};
pub use dyn_aliases::{
    BoxedDemuxReceiver, BoxedMuxSender, BoxedRawReceiver, BoxedRawSender, BoxedReceiver,
    BoxedSender,
};
pub use managed_receive::ManagedReceiveTransport;
pub use mux_sender::{MuxSender, MuxSenderError, MuxSenderStats};
pub use pairing::{
    KlvSample, Pairer, PairerMode, PairerOptions, PairerOutput, PairerStats, VideoSample,
};
pub use raw_receiver::{RawReceiver, RawReceiverStats};
pub use raw_sender::{RawSender, RawSenderConfig, RawSenderStats};
pub use receiver::{Receiver, ReceiverStats};
pub use reconnect::{
    BackoffStrategy, GapBuffer, ManagedTransport, OverflowPolicy, ReconnectPolicy,
};
pub use sender::{Sender, SenderConfig, SenderError, SenderStats, TsFramingMode};

// Re-export the core trait types for caller convenience.
pub use tst_core::transport::{RecvTransport, Transport, TransportCancel, TransportError};

// Re-export the concrete cross-thread shutdown primitive at the crate
// root so FFI binding authors (`srt-jni`, `srt-uniffi`, `tst-pyo3`,
// `tst-c`) have a single import path: `tst_pipeline::CancelHandle`.
//
// `CancelHandle` is a transport-agnostic primitive defined in
// `tst-core`. The pipeline-layer abstraction is `TransportCancel`
// above; shells accept `Option<Arc<dyn TransportCancel + Send + Sync>>`
// via `cancel_handle()`. This re-export lets binding authors name the
// concrete type when they need to construct one or type-erase to it.
// See [`crate`]'s `cancel-handle.md` doc for the full pattern.
pub use tst_core::CancelHandle;
