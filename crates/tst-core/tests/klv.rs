//! Domain harness: KLV substrate + typed MISB set tests (ST 0601/0102/0605/0903) + local KLV fixtures
//! (consolidated from the former per-file tests/*.rs — see tests/MOVEMENT_MAP.md).
//!
//! Each `mod` below is one former top-level integration-test file, now
//! compiled into this single binary. Test bodies are unchanged; only the
//! module path gained a `klv::<file>::` prefix.
#[path = "klv/local_fixtures.rs"]
mod local_fixtures;
#[path = "klv/proptest.rs"]
mod proptest;
#[path = "klv/st0102_via_st0601.rs"]
mod st0102_via_st0601;
#[path = "klv/st0601.rs"]
mod st0601;
#[path = "klv/st0903_standalone_ul.rs"]
mod st0903_standalone_ul;
#[path = "klv/st0903_via_st0601.rs"]
mod st0903_via_st0601;
#[path = "klv/typed_set_proptest.rs"]
mod typed_set_proptest;
