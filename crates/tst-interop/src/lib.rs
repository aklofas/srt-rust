// tst-interop library root.
// Modules land in later tasks.

pub mod fixtures;
// `gen` is a reserved keyword since the 2024 edition (future generator
// syntax) — the module still lives at `src/gen.rs` / is invoked as the
// `gen` CLI subcommand, just spelled `r#gen` at every Rust use site.
pub mod r#gen;
pub mod mux_setup;
pub mod profiles;
pub mod report_types;
pub mod verify;
