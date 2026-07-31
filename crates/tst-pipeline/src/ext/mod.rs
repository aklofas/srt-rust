//! **Stability: Provisional** — see the
//! [API stability reference](https://github.com/aklofas/ts-transformer/blob/main/docs/reference/api-stability.md).
//!
//! Optional extensions to the pipeline shells.
//!
//! Modules under `ext` are **opt-in** — callers reach for them
//! explicitly. Unlike the top-level shells (`MuxSender`, `Sender`,
//! `RawSender`, `DemuxReceiver`, `Receiver`, `RawReceiver`) which are
//! the canonical surface for binding-authors, `ext` modules are
//! domain-specific helpers that ride alongside the core pipeline.
//!
//! Current members:
//!
//! - [`pairing`] — KLV ↔ video PTS-aligned record pairing. Two
//!   strategies: nearest-PTS (with `PairerMode::Realtime` or
//!   `PairerMode::Buffered`) and sample-and-hold (`last_before_pts`).
//!   Opt-in by design — `DemuxReceiver` does not reach for it
//!   automatically, preserving the demux module's decoupled-pairing
//!   posture.

pub mod pairing;
