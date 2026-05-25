# Fuzz seeds

Tracked seed inputs that steer libFuzzer onto interesting code paths from
cold start. Each subdirectory matches a fuzz target name under
`fuzz_targets/`. Seeds are small, hand-built or extracted-from-fixture
byte sequences — they are NOT runtime corpus.

The runtime corpus lives under `crates/*/fuzz/corpus/<target>/` and is
gitignored. To populate `corpus/` from `seeds/` before a fuzz run:

```bash
bash scripts/seed-fuzz-corpora.sh
```

The script is idempotent — it only copies seeds that aren't already in
the target corpus directory, so libFuzzer's accumulated runtime corpus
is preserved.

To add a new seed: drop the file under `seeds/<target>/<name>` and run
the script. Convention is small files (≤ a few KB) with descriptive
names hinting at what code path they exercise (e.g. `pat_real`,
`audio_pusi_rai`, `boundary_188`).
