//! Domain harness: SRT URL parsing and vocabulary coverage
//! (consolidated from the former per-file tests/*.rs — see tests/MOVEMENT_MAP.md).
//!
//! Each `mod` below is one former top-level integration-test file, now
//! compiled into this single binary. Test bodies are unchanged; only the
//! module path gained a `url::<file>::` prefix.
#[path = "url/url_parser.rs"]
mod url_parser;
#[path = "url/url_parser_boundaries.rs"]
mod url_parser_boundaries;
#[path = "url/url_vocabulary_coverage.rs"]
mod url_vocabulary_coverage;
