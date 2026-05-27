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

use pyo3::prelude::*;

pub(crate) fn register(parent: &Bound<'_, PyModule>) -> PyResult<()> {
    // Wave A populates this. For now the module exists but exposes no
    // classes — `import tstrans.srt` succeeds, attribute access fails.
    let m = PyModule::new_bound(parent.py(), "srt")?;
    parent.add_submodule(&m)?;
    Ok(())
}
