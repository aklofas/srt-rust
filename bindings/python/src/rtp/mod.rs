//! `tstrans.rtp` PyO3 bindings. Gated on `feature = "rtp"`.
//!
//! Module structure mirrors the Rust `tst_rtp` crate:
//! - `transport`   — `Sender` + `Receiver` + `SocketStats` + `CancelHandle`
//! - `mux_sender`  — `MuxSender` convenience wrapper (Muxer + Sender)
//! - `demux_receiver` — `DemuxReceiver` convenience wrapper (Demuxer + Receiver)
//! - `client`      — `RtspClient` + `RtspSession` + auth dataclasses
//! - `server`      — `RtspServer` + `MountHandle` (16 push methods)
//!
//! Stage 2 Wave A populates `transport`, `client`, `server` in parallel;
//! Wave B fills `mux_sender` + `demux_receiver` + type stubs; Wave C does
//! integration tests + README. Bootstrap (T19) lands the scaffold.

use pyo3::prelude::*;

pub(crate) mod client;
pub(crate) mod demux_receiver;
pub(crate) mod mux_sender;
pub(crate) mod server;
pub(crate) mod transport;

pub(crate) fn register(parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new_bound(parent.py(), "rtp")?;
    transport::register(&m)?;
    mux_sender::register(&m)?;
    demux_receiver::register(&m)?;
    client::register(&m)?;
    server::register(&m)?;
    parent.add_submodule(&m)?;
    Ok(())
}
