//! TS Transformer core — pure MPEG-TS mux/demux, KLV (MISB ST 0601),
//! codec parameter-set parsers, and transport trait definitions.
//!
//! No I/O, no threads, no transport implementations. The shells that
//! consume the [`Transport`] / [`RecvTransport`] traits live in the
//! companion crate `tst-pipeline`; concrete transport impls (SRT/UDP/
//! RTP/TCP/RTSP) live in their own crates.
//!
//! A few trait-adjacent helpers carry SRT-flavored naming
//! ([`SrtCancelHandle`], [`SocketStats`]) because today's only
//! production transport is libsrt-backed. The code is transport-generic
//! (no `srt-sys` dependency from this crate); the names reflect contract
//! shape, not call sites. Future non-SRT transports may need their own
//! cancel-handle / stats types if the libsrt-flavored contracts don't
//! fit — flagged for post-1.0 review.
//!
//! ## Quick start — round-trip a ST 0601 record
//!
//! ```
//! use tst_core::klv::st0601;
//! use tst_core::UasDatalinkLs;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Build a minimal record. `Default` populates the canonical ST 0601
//! // Universal Label and leaves every typed tag at `None`; `encode_to_vec`
//! // appends Tag 65 (Version) and Tag 1 (Checksum) automatically.
//! let mut ls = UasDatalinkLs::default();
//! ls.timestamp_us = Some(1_700_000_000_000_000);
//!
//! let bytes = st0601::encode_to_vec(&ls)?;
//! let decoded = st0601::decode(&bytes)?;
//! assert_eq!(decoded.timestamp_us, Some(1_700_000_000_000_000));
//! # Ok(())
//! # }
//! ```
//!
//! # Cargo features
//!
//! - `file` (default-on) — enables std::fs-using helpers in `io_file`.
//!   Embedded users without a filesystem disable via
//!   `tst-core = { default-features = false }`.

#![warn(rustdoc::broken_intra_doc_links)]

pub mod cancel;
pub mod codec;
pub mod error;
#[cfg(feature = "file")]
pub mod io_file;
pub mod klv;
pub mod mpegts;
pub mod net;
pub mod transport;
pub mod url;

pub use cancel::SrtCancelHandle;
pub use error::{DemuxError, KlvDecodeError, KlvEncodeError, KlvFieldError, MuxError};
pub use klv::st0601::UasDatalinkLs;
pub use transport::{RecvTransport, SocketStats, Transport, TransportCancel, TransportError};
pub use url::{ParsedUrl, UrlError};
