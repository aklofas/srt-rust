//! TS Transformer core — pure MPEG-TS mux/demux, KLV (MISB ST 0601),
//! codec parameter-set parsers, and transport trait definitions.
//!
//! No I/O, no threads, no transport implementations. The shells that
//! consume the [`Transport`] / [`RecvTransport`] traits live in the
//! companion crate `tst-pipeline`; concrete transport impls (SRT/UDP/
//! RTP/TCP/RTSP) live in their own crates.
//!
//! # Cargo features
//!
//! - `file` (default-on) — enables std::fs-using helpers in `io_file`.
//!   Embedded users without a filesystem disable via
//!   `tst-core = { default-features = false }`.

#![warn(rustdoc::broken_intra_doc_links)]

pub mod codec;
pub mod error;
#[cfg(feature = "file")]
pub mod io_file;
pub mod klv;
pub mod mpegts;
pub mod transport;

pub use error::{KlvDecodeError, KlvEncodeError, KlvFieldError, MuxError, DemuxError};
pub use klv::st0601::UasDatalinkLs;
pub use transport::{RecvTransport, Transport, TransportCancel, TransportError};
