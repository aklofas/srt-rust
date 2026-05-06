//! Safe Rust API for libsrt 1.5.5 — sockets, configuration, error model.
//!
//! This is a thin safety layer on top of `srt-sys`.
//!
//! Quick start:
//!
//! ```no_run
//! use srt_core::srt::SocketBuilder;
//! use std::time::Duration;
//!
//! let mut socket = SocketBuilder::new()
//!     .latency(Duration::from_millis(120))
//!     .connect("127.0.0.1:1234")
//!     .expect("connect");
//!
//! socket.send(b"hello").expect("send");
//! ```

pub mod error;
mod init;
pub mod pipeline;
pub mod srt;

// Re-export tst-core modules so existing users of `srt_core::mpegts`, etc. keep working.
pub use tst_core::codec;
pub use tst_core::klv;
pub use tst_core::mpegts;

// Top-level re-exports for the most common types.
pub use error::{Error, Result};
pub use klv::{Iter, OwnedRawField, RawField, UniversalLabel};
pub use mpegts::demux::{DemuxEvent, Demuxer, StrictMode};
pub use mpegts::mux::{KlvStreamType, Muxer, VideoCodec};
pub use pipeline::{
    BackoffStrategy, ManagedTransport, OverflowPolicy, RawSender, RawSenderConfig, ReconnectPolicy,
    MuxSender, MuxSenderError, SrtTransport, Transport, TransportError, TsFramingMode, Sender,
    SenderConfig, SenderError, SenderStats,
};
pub use srt::url::{SrtUrl, UrlError, UrlOverlay};
pub use srt::{
    Congestion, KeyLength, Listener, ListenerBuilder, ListenerConfig, MaxBandwidth, PacketFilter,
    Passphrase, Role, Socket, SocketBuilder, SocketConfig, Stats, StreamId,
};
