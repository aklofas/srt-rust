//! **Stability: Provisional** — see the
//! [API stability reference](https://github.com/aklofas/ts-transformer/blob/main/docs/reference/api-stability.md).
//!
//! RTSP/1.0 + RTSP/2.0 client (sync facade) — Phase 2.
//! RTSP server (sync facade over internal tokio Runtime) — Phase 3.

pub mod auth;
pub mod client;
pub(crate) mod digest;
pub mod interleaved;
pub mod message;
pub mod server;
