#!/usr/bin/env bash
# Regenerate synthetic subtitle / caption MPEG-TS fixtures for
# tst-core subtitle carriage tests.
#
# Unlike the audio fixtures, these are NOT produced by ffmpeg —
# they're emitted by our own Muxer through the
# `gen-subtitle-fixtures` example. Bootstrap cycle: muxer must work
# to emit them, then they guard against regression in either side.
#
# Run: ./regen.sh
#
# Tools required (locally; not on CI): cargo build environment with
# the libsrt + mbedTLS submodules vendored (SRT_FORCE_VENDORED=1).

set -euo pipefail
DIR="$(cd "$(dirname "$0")" && pwd)"

# Climb to the workspace root: tests/fixtures/subtitles -> crate ->
# crates/ -> repo root (4 levels).
cd "$DIR/../../../.."

SRT_FORCE_VENDORED=1 cargo run -p tst-core --bin gen-subtitle-fixtures --release -- "$DIR"

echo
echo "Regenerated fixtures:"
ls -la "$DIR"/*.ts
