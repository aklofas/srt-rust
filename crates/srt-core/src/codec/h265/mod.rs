//! H.265 / HEVC parameter-set parsers.
//!
//! See [`crate::codec`] for umbrella architecture and design rationale.
//!
//! H.265 parsing is hand-rolled (the `hevc-parser` crate's struct fields
//! are crate-private, and `h265-parser` does not exist); reference
//! sections are noted on each module.

mod bitreader;
mod profile_tier_level;
