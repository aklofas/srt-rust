//! RTSP server — accepts client connections, manages sessions, fans out
//! one Muxer's TS bytes to N connected peers.
//!
//! Phase 3 — populated by Wave A through Wave G tasks. This module
//! declaration file ships in the bootstrap task (Task 1) so subsequent
//! parallel-wave subagents can target individual submodules without
//! the lib.rs path being a moving target.

pub mod auth;
pub mod builder;
pub mod fanout;
pub mod handlers;
pub mod interleaved_pump;
pub mod listener;
pub mod mount;
pub mod multicast;
pub mod runtime;
pub mod session;
#[cfg(feature = "tls")]
pub mod tls;
