//! PyO3 bindings for the ts-transformer Rust workspace.
//!
//! The compiled cdylib is imported from Python as `tstrans._native`;
//! the top-level `tstrans/__init__.py` re-exports the public surface
//! into submodules so users `from tstrans.mpegts import Muxer`.
//!
//! Exports `__version__`, the exception handles, and the PyClass
//! wrappers for the `mpegts`, `klv`, and `codec` submodules.

mod codec;
mod errors;
mod klv;
mod mpegts;
mod mux;
#[cfg(feature = "rtp")]
mod rtp;
#[cfg(feature = "srt")]
mod srt;
// Plan A5b — udp / tcp / hls / rist transport bindings (default-on).
#[cfg(feature = "udp")]
mod udp;
#[cfg(feature = "tcp")]
mod tcp;
#[cfg(feature = "hls")]
mod hls;
#[cfg(feature = "rist")]
mod rist;
#[cfg(feature = "pipeline")]
mod pipeline;

use pyo3::prelude::*;

// `_py` is prefixed because it is not used directly in this
// registration shell. CI runs `clippy -D warnings` so an unprefixed
// unused parameter would fail the workspace.
#[pymodule]
fn _native(_py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add_function(wrap_pyfunction!(errors::raise_mux_error_for_test, m)?)?;
    #[cfg(feature = "rtp")]
    {
        m.add_function(wrap_pyfunction!(errors::raise_rtsp_error_for_test, m)?)?;
        m.add_function(wrap_pyfunction!(errors::raise_rtp_error_for_test, m)?)?;
    }
    #[cfg(feature = "srt")]
    {
        m.add_function(wrap_pyfunction!(errors::raise_srt_error_for_test, m)?)?;
    }
    #[cfg(feature = "udp")]
    {
        m.add_function(wrap_pyfunction!(errors::raise_udp_error_for_test, m)?)?;
    }
    #[cfg(feature = "tcp")]
    {
        m.add_function(wrap_pyfunction!(errors::raise_tcp_error_for_test, m)?)?;
    }
    #[cfg(feature = "hls")]
    {
        m.add_function(wrap_pyfunction!(errors::raise_hls_error_for_test, m)?)?;
    }
    #[cfg(feature = "rist")]
    {
        m.add_function(wrap_pyfunction!(errors::raise_rist_error_for_test, m)?)?;
    }
    mpegts::register(m)?;
    klv::register(m)?;
    // Stream handle newtypes. Registered here (not in mux::register)
    // because they were the first mux surface to land in src/mux.rs.
    m.add_class::<crate::mux::PyVideoStreamHandle>()?;
    m.add_class::<crate::mux::PyAudioStreamHandle>()?;
    m.add_class::<crate::mux::PyKlvStreamHandle>()?;
    m.add_class::<crate::mux::PySubtitleStreamHandle>()?;
    m.add_class::<crate::mux::PyDataStreamHandle>()?;
    // Program-level config + builder.
    m.add_class::<crate::mux::PyMuxerProgramConfig>()?;
    m.add_class::<crate::mux::PyMuxerProgramConfigBuilder>()?;
    // Top-level muxer config + builder.
    m.add_class::<crate::mux::PyMuxerConfig>()?;
    m.add_class::<crate::mux::PyMuxerConfigBuilder>()?;
    // Muxer base (init + pull + pending + capacity).
    m.add_class::<crate::mux::PyMuxer>()?;
    // MuxerStats snapshot (StreamCodecStats is pure Python —
    // constructed by `Muxer.stream_codec_stats` per call).
    m.add_class::<crate::mux::PyMuxerStats>()?;
    // codec submodule — shared types, NalUnit, Obu, and per-codec
    // PyClasses.
    codec::register(m)?;
    // rtp submodule — RTP + RTSP bindings (Wave A populates the
    // contents).
    #[cfg(feature = "rtp")]
    rtp::register(m)?;
    // srt submodule — SRT transport bindings (Wave A populates the
    // contents).
    #[cfg(feature = "srt")]
    srt::register(m)?;
    // Plan A5b — udp / tcp / hls / rist submodules (waves populate contents).
    #[cfg(feature = "udp")]
    udp::register(m)?;
    #[cfg(feature = "tcp")]
    tcp::register(m)?;
    #[cfg(feature = "hls")]
    hls::register(m)?;
    #[cfg(feature = "rist")]
    rist::register(m)?;
    // pipeline submodule — the ext::pairing PairingDemuxer composite,
    // exposed as tstrans.pipeline.Pairer.
    #[cfg(feature = "pipeline")]
    pipeline::register(m)?;
    Ok(())
}
