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

# Copy per-target .options files where present.
for options_file in oss-fuzz/targets/*.options; do
  [ -f "$options_file" ] || continue
  options_name=$(basename "$options_file")
  cp "$options_file" "$OUT/$options_name"
done


# Copy the shared KLV dictionary if present.
if [ -f oss-fuzz/targets/klv.dict ]; then
  # libFuzzer's -dict flag takes a single file. Each KLV target gets the
  # dict via its corresponding .options file's `dict = ` line — but since
  # we ship one dict and want it picked up automatically, naming it
  # <target>.dict makes libFuzzer find it without an explicit option.
  for tgt in klv_iter klv_st0601_decode klv_st0102_decode klv_st0903_decode; do
    cp oss-fuzz/targets/klv.dict "$OUT/${tgt}.dict"
  done
fi

# Seed corpus — derived from existing committed fixtures.
#
# Convention: $OUT/<target>_seed_corpus.zip contains seed inputs.
# libFuzzer auto-loads this zip when started by OSS-Fuzz's wrapper.

# klv_st0601_decode: 4 hand-crafted synthetic ST 0601 fixtures.
if [ -d crates/tst-core/tests/fixtures/st0601 ]; then
  (cd crates/tst-core/tests/fixtures/st0601 && zip -j "$OUT/klv_st0601_decode_seed_corpus.zip" *.klv) || \
    echo "WARN: klv_st0601_decode seed corpus zip failed"
fi

# demux_feed, demux_psi, demux_pes_reassembly, ts_parser:
# plan #52's regression fixtures. All four parse the same TS-packet
# bytestream, so share the corpus.
if [ -d crates/tst-core/tests/fixtures/regression ]; then
  for tgt in demux_feed demux_psi demux_pes_reassembly ts_parser; do
    (cd crates/tst-core/tests/fixtures/regression && \
     ls *.bin >/dev/null 2>&1 && \
     zip -j "$OUT/${tgt}_seed_corpus.zip" *.bin) || \
      echo "INFO: no regression fixtures yet for $tgt (corpus_to_fixture not used yet)"
  done
fi

# Confirm the expected count made it to $OUT/.
shipped=$(ls "$OUT/" | wc -l)
echo "INFO: shipped $shipped fuzz drivers to \$OUT"
