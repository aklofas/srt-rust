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
//! use std::time::Duration;
//!
//! let mut socket = SocketBuilder::new()
//!     .latency(Duration::from_millis(120))
//!     .connect("127.0.0.1:1234")
//!     .expect("connect");
//!
//! socket.send(b"hello").expect("send");
//! ```

#![warn(rustdoc::broken_intra_doc_links)]

pub mod addr;
pub mod builder;
pub mod cancel;
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
pub use cancel::CancelHandle;
pub use config::{ListenerConfig, SocketConfig};
pub use error::{AcceptError, BindError, ConnectError, Error, RecvError, Result, SendError};
pub use listener::Listener;
pub use options::{Congestion, KeyLength, MaxBandwidth, PacketFilter, Passphrase, Role, StreamId};
pub use socket::{Socket, Stats};
pub use transport::SrtTransport;
pub use url::{SrtUrl, UrlError, UrlOverlay};
