//! Typed codec parameter-set parsers.
//!
//! Stateless parsers for video codec parameter sets (SPS / VPS / PPS).
//! Each codec lives in its own submodule with consistent function shape.
//! Consumers receive raw NAL units from [`crate::mpegts::demux`] and call
//! the parser explicitly when typed fields are needed.
//!
//! Shipped this slice: H.264 ([`h264`]) and H.265 ([`h265`]).
//! Future slices in the same umbrella: AV1, H.266, audio framing,
//! subtitle parsers — each will appear here as `codec::<name>`.

pub mod h264;
pub mod h265;
