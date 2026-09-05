# Capture a regression fixture from a corpus `.ts` file

> **When to use this:** The gitignored corpus surfaces a parser or demuxer bug and you want to preserve a minimal reproducer as a committed regression test.

> **Related:**
> - [guides/mpegts-demux.md](/docs/guides/mpegts-demux.md) — the demuxer surface the fixture exercises
> - `crates/tst-core/tests/fixtures/regression/README.md` — full workflow

When the gitignored corpus surfaces a parser or demuxer bug, preserve a
minimal reproducer as a committed regression test:

```bash
cargo run -p tst-core --bin corpus-to-fixture -- \
  --input ~/path/to/sensitive_sample.ts \
  --pid 0x1011 \
  --packets 1000..2000 \
  --out crates/tst-core/tests/fixtures/regression/<slug>.bin \
  --emit-shim
```

This writes the filtered TS packets to `tests/fixtures/regression/<slug>.bin`
and a Cargo integration test at `tests/regression_<slug>.rs` that
smoke-tests the fixture through the demuxer. Add domain-specific
assertions below the smoke-test before committing.

See `crates/tst-core/tests/fixtures/regression/README.md` for the full workflow.
