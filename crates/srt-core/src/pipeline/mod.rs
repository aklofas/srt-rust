// crates/srt-core/src/pipeline/mod.rs
//! Composition layer wiring `mpegts::mux::Muxer`, `klv::*`, and `srt::Socket`
//! into ergonomic sender and receiver shells for the canonical
//! platform-to-ground-station send path and ground-station receive path.
//!
//! **Send side:** three sender shells (`Sender`, `TsSender`, `RawSender`) are
//! generic over a `Transport`; the `ManagedTransport` decorator (in
//! `pipeline::reconnect`) wraps any of them with reconnect + gap buffering.
//!
//! **Receive side:** receive shells (`RawReceiver`, and the forthcoming
//! `TsReceiver` / `Receiver`) are generic over a `RecvTransport`. `SrtTransport`
//! implements both `Transport` and `RecvTransport`.

pub mod raw_receiver;
pub mod raw_sender;
pub mod reconnect;
pub mod recv_transport;
pub mod sender;
pub mod srt_transport;
pub mod transport;
pub mod ts_receiver;
pub mod ts_sender;

pub use raw_receiver::RawReceiver;
pub use raw_sender::{RawSender, RawSenderConfig};
pub use reconnect::{BackoffStrategy, ManagedTransport, OverflowPolicy, ReconnectPolicy};
pub use recv_transport::RecvTransport;
pub use sender::{Sender, SenderError};
pub use srt_transport::SrtTransport;
pub use transport::{Transport, TransportError};
pub use ts_receiver::TsReceiver;
pub use ts_sender::{TsFramingMode, TsSender, TsSenderConfig, TsSenderError, TsSenderStats};
