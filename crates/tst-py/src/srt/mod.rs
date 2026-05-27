//! `tstrans.srt` — SRT transport bindings.
//!
//! See `docs/specs/2026-05-27-tst-py-srt-design.md` for the surface
//! shape. Mirrors the `tstrans.rtp` pattern (Phase 4 Stage 2) — same
//! GIL-release boundaries, same bytes-like extraction, same error
//! mapping shape.
//!
//! Submodules:
//! - `transport` (Wave A T2): Sender, Receiver, SocketStats, SrtStats, CancelHandle
//! - `lowlevel` (Wave A T3): Socket, Listener, Builder
//! - `convenience` (Wave B T5): MuxSender, DemuxReceiver
//! - `policy` (Wave B T6): ReconnectPolicy, BackoffStrategy, OverflowPolicy
//! - `managed` (Wave C T7+T8): ManagedSender, ManagedReceiver, ManagedMuxSender, ManagedDemuxReceiver
//!
//! Error mapping lives in `crate::errors::make_srt_error` (Wave A T4).

mod transport;

use pyo3::prelude::*;

pub(crate) fn register(parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new_bound(parent.py(), "srt")?;
    transport::register(&m)?;
    parent.add_submodule(&m)?;
    Ok(())
}
