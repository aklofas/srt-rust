//! MPEG-TS muxing — sender-side TS packetization for H.264/H.265 + ST 0601 KLV.
//!
//! ## Quick start
//!
//! ```no_run
//! use srt_core::mpegts::mux::{Config, Muxer};
//!
//! let mut mux = Muxer::new(Config::default()).unwrap();
//!
//! // Push one access unit (Annex-B framed) per frame:
//! # let access_unit_bytes = vec![0u8; 0];
//! # let pts = 0;
//! mux.push_video(&access_unit_bytes, pts, /*key_frame=*/ true).unwrap();
//!
//! // Push KLV metadata at any cadence:
//! # let klv_bytes = vec![0u8; 0];
//! mux.push_klv(&klv_bytes, pts).unwrap();
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
//! - ST 1402 KLV — both `PrivateData` (0x06) and `SynchronousMetadata` (0x15)
//! - ST 1910 AU cell wrapping is in [`crate::klv::st1910`], not the muxer —
//!   wrap KLV bytes there before calling [`mux::Muxer::push_klv`] when you want
//!   the per-frame timestamp embedded
//! - VBR output, no null padding
//! - Annex-B input; one access unit per [`mux::Muxer::push_video`] call

pub mod common;
pub mod demux;
pub mod mux;
pub mod stats;

pub use stats::StreamStats;
