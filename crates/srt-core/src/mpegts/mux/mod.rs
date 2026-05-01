//! Sender-side MPEG-TS muxer.
//!
//! Public surface filled in by Tasks 7-8. Internal helpers in `ts`, `psi`,
//! `pes` submodules.

pub(crate) mod pes;
pub(crate) mod psi;
pub(crate) mod ts;
