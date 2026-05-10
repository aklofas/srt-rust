//! TS Transformer SRT transport — safe libsrt wrapper, Socket / Listener /
//! Builder, URL parsing, Transport + RecvTransport implementations.
//!
//! This crate provides the SRT-specific concrete types. The transport
//! traits themselves live in [`tst_core`]; the transport-agnostic
//! Sender/Receiver shells live in `tst_pipeline`.
//!
//! Quick start:
//!
//! ```no_run
//! use tst_srt::SocketBuilder;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let mut b = SocketBuilder::new();
//! b.latency_ms(120);
//! let mut socket = b.connect("127.0.0.1:1234")?;
//!
//! socket.send(b"hello")?;
//! # Ok(())
//! # }
//! ```

#![warn(rustdoc::broken_intra_doc_links)]

pub mod addr;
pub mod builder;
pub mod config;
pub mod error;
pub mod init;
pub mod listener;
pub mod options;
pub mod socket;
pub mod transport;
pub mod url;

// Top-level re-exports for the most common types.
pub use builder::{ListenerBuilder, SocketBuilder};
pub use config::{ListenerConfig, SocketConfig};
pub use error::{AcceptError, BindError, ConnectError, Error, RecvError, Result, SendError};
pub use listener::Listener;
pub use options::{Congestion, KeyLength, MaxBandwidth, PacketFilter, Passphrase, Role, StreamId};
pub use socket::{Socket, Stats};
pub use transport::SrtTransport;
// `CancelHandle` is a transport-agnostic primitive defined in `tst-core`;
// re-exported here for backwards compatibility so `tst_srt::CancelHandle`
// keeps working at existing call sites.
pub use tst_core::CancelHandle;
pub use url::{SrtUrl, UrlError, UrlOverlay};
