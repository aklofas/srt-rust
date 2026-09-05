# Regression fixtures captured from the local corpus

Files here are minimal TS-packet sub-sequences that triggered a parser
or demuxer bug in `tst-core`. Each fixture has:

- `<slug>.bin` — the captured bytes (188-byte-aligned TS packets).
- `../../regression_<slug>.rs` — an auto-generated Cargo integration test
  that `include_bytes!`s the bin and runs a smoke-test through `Demuxer`.

After capturing a fixture, the workflow is:

1. Run `corpus-to-fixture` against the original (gitignored) `.ts` file:
   ```bash
   cargo run -p tst-core --bin corpus-to-fixture -- \
     --input ../path/to/sensitive.ts \
     --pid 0x1011 \
     --packets 1234..5678 \
     --out crates/tst-core/tests/fixtures/regression/<slug>.bin \
     --emit-shim
   ```
2. Open `tests/regression_<slug>.rs` and add domain-specific assertions
   below the smoke-test — what specific demuxer event sequence, KLV
   field value, codec parameter, etc. the original bug got wrong.
3. Run `cargo test -p tst-core --test regression_<slug>` to confirm the
   shim compiles and runs.
4. Commit `<slug>.bin` + `regression_<slug>.rs` together. NEVER commit
   the original `.ts` source — it lives in the gitignored corpus.

## Slug naming

Slugs must match `[a-z0-9_]+`. Convention: `<topic>_<symptom>`. Examples:
`pat_multi_section_panic`, `klv_st0601_tag74_truncated`, `h265_rps_overflow`.

## When to refresh

If a parser bugfix changes the *meaning* of a fixture's bytes (e.g. a
previously-failing parse now succeeds with different output), the shim's
domain-specific assertions need updating to match the new correct
behavior. The smoke-test ("no panic + at least one event") is a baseline
that doesn't usually need refreshing.
