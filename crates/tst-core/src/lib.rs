//! TS Transformer core — pure MPEG-TS mux/demux, KLV (MISB ST 0601),
//! codec parameter-set parsers, and transport trait definitions.
//!
//! No I/O, no threads, no transport implementations. The shells that
//! consume the [`Transport`] / [`RecvTransport`] traits live in the
//! companion crate `tst-pipeline`; concrete transport impls (SRT/UDP/
//! RTP/TCP/RTSP) live in their own crates.
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
pub mod transport;

pub use cancel::CancelHandle;
pub use error::{DemuxError, KlvDecodeError, KlvEncodeError, KlvFieldError, MuxError};
pub use klv::st0601::UasDatalinkLs;
pub use transport::{RecvTransport, Transport, TransportCancel, TransportError};
