//! Test helpers shared across the workspace's `tests/` integration suites.
//!
//! Modules are added by Phase 5 Tasks 8 / 9 / 10. Crate is `publish = false`
//! and lives only in `[dev-dependencies]`; no shipping artifact contains it.

pub mod synthetic_nal;
pub mod ts_parser;
