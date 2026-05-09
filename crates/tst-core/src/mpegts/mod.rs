//! MPEG-TS muxing — sender-side TS packetization for H.264/H.265 + ST 0601 KLV.
//!
//! ## Quick start
//!
//! ```no_run
//! use tst_core::mpegts::mux::{MuxerConfig, Muxer};
//!
//! let mut mux = Muxer::new(MuxerConfig::default()).unwrap();
//!
//! // Push one access unit (Annex-B framed) per frame:
//! # let access_unit_bytes = vec![0u8; 0];
//! # let pts = 0;
//! mux.push_video(&access_unit_bytes, pts, /*key_frame=*/ true).unwrap();
//!
//! // Push KLV metadata at any cadence:
//! # let klv_bytes = vec![0u8; 0];
//! mux.push_klv(&klv_bytes, pts, 0x00).unwrap();
//!
//! // Drain TS packets into your transport:
//! let mut buf = [0u8; 1316]; // SRT live-mode payload size = 7 × 188
//! loop {
//!     let n = mux.pull(&mut buf);
//!     if n == 0 { break; }
//!     // socket.send(&buf[..n]).unwrap();
//! }
//! ```
//!
//! ## What's in scope
//!
//! - Single-program TS, one video PID, one KLV PID, no audio
//! - H.264 (stream_type 0x1B) and H.265 (stream_type 0x24)
//! - ST 1402 KLV — both `PrivateData` (0x06) and `SynchronousMetadata` (0x15);
//!   sync streams auto-wrap inside [`mux::Muxer::push_klv`] with a 5-byte
//!   `Metadata_AU_cell` header per ITU-T H.222.0 V9 §2.12.4.2 (see
//!   [`crate::mpegts::au_cell`])
//! - VBR output, no null padding
//! - Annex-B input; one access unit per [`mux::Muxer::push_video`] call

pub mod au_cell;
pub mod common;
pub mod demux;
pub mod descriptors;
pub mod mux;
pub mod stats;

pub use stats::StreamStats;
