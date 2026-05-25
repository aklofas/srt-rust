#!/usr/bin/env bash
# Seed libFuzzer corpus directories from the tracked seeds/ trees.
#
# Project policy gitignores `crates/*/fuzz/corpus/` because libFuzzer
# populates them at runtime. To still ship initial seed inputs that
# steer libFuzzer onto interesting code paths from cold start, seeds
# live under `crates/*/fuzz/seeds/<target>/`. This script copies them
# into the corpus tree without clobbering accumulated runtime corpus.
#
# Idempotent — safe to run repeatedly. Used by:
#   - local pre-push fuzz check (recommended before `cargo +nightly fuzz check`)
#   - CI fuzz job (`.github/workflows/ci.yml`)
#
# Exit codes: 0 = ok; non-zero = a seeds/ entry exists but cp failed.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"

shopt -s nullglob

for fuzz_crate in "$ROOT"/crates/*/fuzz; do
    seeds_root="$fuzz_crate/seeds"
    [ -d "$seeds_root" ] || continue
    for target_dir in "$seeds_root"/*/; do
        target=$(basename "$target_dir")
        corpus_dir="$fuzz_crate/corpus/$target"
        mkdir -p "$corpus_dir"
        # Preserve libFuzzer's runtime corpus growth: only copy seeds that
        # aren't already in the corpus. Portable across GNU coreutils
        # versions (newer ones deprecate `cp -n`) and macOS bash 3.2.
        for seed in "$target_dir"*; do
            [ -f "$seed" ] || continue
            dest="$corpus_dir/$(basename "$seed")"
            [ -e "$dest" ] || cp "$seed" "$dest"
        done
    done
done
