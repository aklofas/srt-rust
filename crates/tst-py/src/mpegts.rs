//! PyO3 wrappers for `tst_core::mpegts::Demuxer` + `DemuxEvent`.
//!
//! Translation strategy: each Rust `DemuxEvent` variant is converted
//! to an instance of a Python-side subclass under
//! `tstrans.mpegts.DemuxEvent.*` via `convert_event(py, ...)`. Support
//! types (`StreamId`, `StreamInfo`, `ProgramMap`) are built from
//! Python-side dataclasses defined in `tstrans/mpegts.py`.
//!
//! Phase 2 ships: `Demuxer` PyClass + event conversion for all 6
//! Rust DemuxEvent variants. Sample.payload exposed as raw `bytes`;
//! typed NAL/OBU access lands in Phase 5.

// Phase 2 Task 7 fills this module.
