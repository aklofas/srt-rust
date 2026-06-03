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
  for tgt in klv_st0601_decode klv_st0102_decode klv_st0903_decode; do
    cp oss-fuzz/targets/klv.dict "$OUT/${tgt}.dict"
  done
fi

# Seed corpus packaging — unified loop.
#
# Per-target precedence (zip merges all sources for a target):
#   1. crates/tst-core/tests/fixtures/<source>/  (fixture-derived; do not edit)
#   2. crates/<crate>/fuzz/seeds/<target>/       (committed synthetic seeds — single source of truth)

# Helper: zip-or-append for a single target.
zip_seeds() {
  local target="$1"; shift
  local out_zip="$OUT/${target}_seed_corpus.zip"
  for src_dir in "$@"; do
    if [ -d "$src_dir" ] && ls "$src_dir"/* >/dev/null 2>&1; then
      (cd "$src_dir" && zip -j -q "$out_zip" *)
    fi
  done
}

# Fixture-derived seeds.
zip_seeds klv_st0601_decode      crates/tst-core/tests/fixtures/st0601
zip_seeds demux_feed             crates/tst-core/tests/fixtures/regression
zip_seeds demux_psi              crates/tst-core/tests/fixtures/regression
zip_seeds demux_pes_reassembly   crates/tst-core/tests/fixtures/regression
zip_seeds ts_parser              crates/tst-core/tests/fixtures/regression

# Committed synthetic seeds — canonical under crates/<crate>/fuzz/seeds/<target>/.
zip_seeds mpegts_au_cell_read       crates/tst-core/fuzz/seeds/mpegts_au_cell_read
zip_seeds audio_frame_iter          crates/tst-core/fuzz/seeds/audio_frame_iter
zip_seeds url_parse                 crates/tst-srt/fuzz/seeds/url_parse
zip_seeds mux_pull                  crates/tst-core/fuzz/seeds/mux_pull
zip_seeds mux_push_klv              crates/tst-core/fuzz/seeds/mux_push_klv
zip_seeds mux_push_video            crates/tst-core/fuzz/seeds/mux_push_video
zip_seeds parse_parameter_sets      crates/tst-core/fuzz/seeds/parse_parameter_sets
zip_seeds parse_av1_sequence_header crates/tst-core/fuzz/seeds/parse_av1_sequence_header

# Confirm the expected count made it to $OUT/.
shipped=$(ls "$OUT/" | wc -l)
echo "INFO: shipped $shipped fuzz drivers to \$OUT"
