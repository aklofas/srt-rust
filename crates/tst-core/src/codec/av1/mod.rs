//! AV1 (AOM Bitstream Spec) OBU parsers.
//!
//! Hand-rolled per the AV1 Bitstream & Decoding Process Specification.
//! Reuses internal LEB128 primitives for OBU size encoding
//! and AV1's `uvlc` reads.
//!
//! ## Spec coverage
//!
//! Parsed per the AV1 Bitstream & Decoding Process Specification:
//! - Sequence Header OBU: profile / level / tier / dimensions /
//!   bit depth / chroma format / color info / still-picture flags.
//! - Frame Header OBU (light): frame_type / show_frame /
//!   show_existing_frame.
//! - OBU stream collector [`parse_obu_stream`]: walks `Vec<Obu>`,
//!   partial-success-tolerant.
//!
//! ## Not parsed (deferred)
//!
//! - Per-frame size override in Frame Header — full reference frame
//!   management not surfaced; `frame_size` is always `None`.
//!   See `docs/deferred-features.md`.
//! - Tile Group OBUs (pass-through unparsed via `mpegts::demux::event::Obu`).
//! - Metadata OBUs (pass-through).
//! - Padding OBUs.

mod decode;
mod model;

#[cfg(test)]
mod tests;

pub use decode::{parse_frame_header_light, parse_obu_stream, parse_sequence_header};
pub use model::{Av1FrameHeaderLight, Av1ObuStream, Av1SequenceHeader};

// Back-compat re-export: `mpegts::demux::payload` imports
// `crate::codec::av1::leb128::read_leb128`. The decode/ reorg moves the
// source file but this re-export keeps the consumer path unchanged.
pub(crate) use decode::leb128;
