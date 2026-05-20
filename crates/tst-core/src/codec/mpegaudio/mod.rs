//! MPEG-1 / MPEG-2 / MPEG-2.5 audio frame iterator.
//!
//! See [`crate::codec`] for umbrella architecture and design rationale.
//!
//! ## Spec coverage
//!
//! Parsed per ISO/IEC 11172-3 (MPEG-1 Audio) §2.4, ISO/IEC 13818-3
//! (MPEG-2 Lower Sampling Frequencies), and the de-facto MPEG-2.5
//! half-rate extension:
//! - Layer I, II, III.
//! - Version: MPEG-1, MPEG-2, MPEG-2.5.
//! - Per-frame: bitrate, sample_rate, channel_mode, channels,
//!   frame_length, samples_per_frame, has_crc.
//!
//! ## Not parsed (deferred)
//!
//! - Side information / main data beyond the 4-byte header.
//! - Psychoacoustic model fields.
//! - ID3 / APEv2 tag detection (tag bytes are surfaced as parse errors
//!   and terminate the iterator).

mod decode;
mod model;
mod tables;

pub use decode::{frames, frames_with_resync};
pub use model::{ChannelMode, Frame, FrameOwned, Frames, Layer, Version};
