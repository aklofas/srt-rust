//! AV1 (AOM Bitstream Spec) OBU parsers.
//!
//! Hand-rolled per the AV1 Bitstream & Decoding Process Specification.
//! Reuses the LEB128 primitives in [`leb128`] for OBU size encoding
//! and AV1's `uvlc` reads.
//!
//! ## Scope
//!
//! - Sequence Header OBU: profile / level / tier / dimensions /
//!   bit depth / chroma format / color info / still-picture flags.
//! - Frame Header OBU (light): frame_type / show_frame /
//!   show_existing_frame. Per-frame size override + full reference
//!   frame management deferred — see `docs/deferred-features.md`.
//! - Other OBU types (Tile Group, Metadata, Padding) pass through
//!   unparsed; see `mpegts::demux::event::Obu`.

pub(crate) mod bitreader;
pub mod frame_header;
pub mod leb128;
pub mod obu_stream;
pub mod sequence_header;

pub use frame_header::{Av1FrameHeaderLight, parse_frame_header_light};
pub use obu_stream::{Av1ObuStream, parse_obu_stream};
pub use sequence_header::{Av1SequenceHeader, parse_sequence_header};
