#!/bin/bash -eu
# OSS-Fuzz build script for ts-transformer.
#
# Invoked inside the OSS-Fuzz base-builder-rust container by:
#   docker run -v $OUT:/out -e SANITIZER=address \
#     gcr.io/oss-fuzz/ts-transformer compile
#
# Responsibilities:
#   1. Build all 16 cargo-fuzz harnesses (tst-core: 15, tst-srt: 1) under
#      libFuzzer with the SANITIZER env-var honored by cargo-fuzz.
#   2. Copy each driver binary to $OUT/<target_name>.
#
# Seed corpora, dictionaries, and .options files are emitted in later
# build.sh revisions — see oss-fuzz/README.md.
set -euo pipefail

# Vendored libsrt + mbedTLS build path, matches CI.
export SRT_FORCE_VENDORED=1

# cargo-fuzz target dir (per cargo-fuzz convention).
TARGET_DIR=target/x86_64-unknown-linux-gnu/release

# Build tst-core fuzz targets.
pushd crates/tst-core
cargo +nightly fuzz build --release
popd

# Build tst-srt fuzz targets.
pushd crates/tst-srt
cargo +nightly fuzz build --release
popd

# Copy tst-core fuzz drivers to $OUT/.
for target_src in crates/tst-core/fuzz/fuzz_targets/*.rs; do
  target_name=$(basename "$target_src" .rs)
  bin_path="crates/tst-core/fuzz/$TARGET_DIR/$target_name"
  if [ -x "$bin_path" ]; then
    cp "$bin_path" "$OUT/$target_name"
  else
    echo "WARN: tst-core fuzz binary missing for $target_name (expected $bin_path)"
  fi
done

# Copy tst-srt fuzz drivers to $OUT/.
for target_src in crates/tst-srt/fuzz/fuzz_targets/*.rs; do
  target_name=$(basename "$target_src" .rs)
  bin_path="crates/tst-srt/fuzz/$TARGET_DIR/$target_name"
  if [ -x "$bin_path" ]; then
    cp "$bin_path" "$OUT/$target_name"
  else
    echo "WARN: tst-srt fuzz binary missing for $target_name (expected $bin_path)"
  fi
done

# Confirm the expected count made it to $OUT/.
shipped=$(ls "$OUT/" | wc -l)
echo "INFO: shipped $shipped fuzz drivers to \$OUT"
