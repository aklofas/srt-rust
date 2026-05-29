//! Domain harness: conformance-corpus + demux error/recovery discrimination tests
//! (consolidated from the former per-file tests/*.rs — see tests/MOVEMENT_MAP.md).
//!
//! Each `mod` below is one former top-level integration-test file, now
//! compiled into this single binary. Test bodies are unchanged; only the
//! module path gained a `conformance::<file>::` prefix.
#[path = "conformance/conformance.rs"]
mod conformance;
#[path = "conformance/demux_error_discrimination.rs"]
mod demux_error_discrimination;
#[path = "conformance/demux_malformed_pes_recovery.rs"]
mod demux_malformed_pes_recovery;
