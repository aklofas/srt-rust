// crates/srt-core/src/pipeline/mod.rs
//! Composition layer wiring `mpegts::mux::Muxer`, `klv::*`, and `srt::Socket`
//! into ergonomic sender shells for the canonical platform-to-ground-station
//! send path.
//!
//! All three sender shells (`Sender`, `TsSender`, `RawSender`) are generic
//! over a `Transport`; the `ManagedTransport` decorator (in
//! `pipeline::reconnect`) wraps any of them with reconnect + gap buffering.

pub mod raw_sender;
pub mod reconnect;
pub mod sender;
pub mod srt_transport;
pub mod transport;
pub mod ts_sender;

pub use raw_sender::{RawSender, RawSenderConfig};
pub use reconnect::{BackoffStrategy, ManagedTransport, OverflowPolicy, ReconnectPolicy};
pub use sender::{Sender, SenderError};
pub use srt_transport::SrtTransport;
pub use transport::{Transport, TransportError};
pub use ts_sender::{TsFramingMode, TsSender, TsSenderConfig, TsSenderError, TsSenderStats};
