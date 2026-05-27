//! `tstrans.srt` — SRT transport bindings.
//!
//! See `docs/specs/2026-05-27-tst-py-srt-design.md` for the surface
//! shape. Mirrors the `tstrans.rtp` pattern (Phase 4 Stage 2) — same
//! GIL-release boundaries, same bytes-like extraction, same error
//! mapping shape.
//!
//! Submodules:
//! - `transport`      (Wave A T2): Sender, Receiver, SocketStats, SrtStats, CancelHandle
//! - `lowlevel`       (Wave A T3): Socket, Listener, Builder
//! - `mux_sender`     (Wave B T5): MuxSender convenience wrapper
//! - `demux_receiver` (Wave B T5): DemuxReceiver convenience wrapper
//! - `policy`         (Wave B T6): ReconnectPolicy, BackoffStrategy, OverflowPolicy
//! - `managed`        (Wave C T7+T8): ManagedSender, ManagedReceiver, ManagedMuxSender, ManagedDemuxReceiver
//!
//! Error mapping lives in `crate::srt::errors` (Wave A T4) — typed
//! `*_to_pyerr` helpers consolidate every Rust enum that flows through
//! the surface (UrlError / ConnectError / BindError / AcceptError /
//! IoError / TransportError) into the 8-variant `SrtErrorKind`.

pub(crate) mod demux_receiver;
pub(crate) mod errors;
mod lowlevel;
pub(crate) mod mux_sender;
pub(crate) mod policy;
mod transport;

use pyo3::prelude::*;

pub(crate) fn register(parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new_bound(parent.py(), "srt")?;
    transport::register(&m)?;
    lowlevel::register(&m)?;
    mux_sender::register(&m)?;
    demux_receiver::register(&m)?;
    policy::register(&m)?;
    parent.add_submodule(&m)?;
    Ok(())
}
