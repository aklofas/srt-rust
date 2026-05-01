// crates/srt-core/src/mpegts/mod.rs
//! MPEG-TS muxing — sender-side TS packetization for H.264/H.265 + ST 0601 KLV.
//!
//! Two submodules:
//!
//! - **`common`** — types shared with the eventually-deferred `mpegts::demux`:
//!   `StreamType`, descriptor and PID constants, 90 kHz / 27 MHz timestamp
//!   newtypes, hand-rolled CRC-32/MPEG-2.
//! - **`mux`** — the sender-side `Muxer`, plus internal `ts`/`psi`/`pes`
//!   helpers. Public surface: `Config`, `Muxer`, `VideoCodec`, `KlvStreamType`.
//!
//! See the design document for architecture and decisions:
//! `docs/specs/2026-05-01-srt-core-mpegts-mux-design.md` (in the parent
//! workspace, not in this repo).

pub mod common;
pub mod mux;
