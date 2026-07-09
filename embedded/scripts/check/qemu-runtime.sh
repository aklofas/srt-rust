#!/usr/bin/env bash
# Local mirror of the CI QEMU runtime gate.
#
# Runs the tst-core muxer on a Cortex-M4 under QEMU and asserts it byte-matches
# the committed video-roundtrip golden (see embedded/baremetal-qemu/). Skips
# gracefully when qemu-system-arm is not installed, so pre-push runs on
# machines without QEMU do not fail — same optional-tool pattern as the
# ffmpeg-dependent tests.
set -euo pipefail
cd "$(dirname "$0")/../../.."

REQUIRE="${QEMU_RUNTIME_REQUIRE_TOOLS:-0}"
if ! command -v qemu-system-arm >/dev/null 2>&1; then
  if [ "$REQUIRE" = "1" ]; then echo "FATAL: required tool 'qemu-system-arm' not installed (QEMU_RUNTIME_REQUIRE_TOOLS=1)"; exit 1; fi
  echo "SKIP: qemu-system-arm not installed (apt install qemu-system-arm)"; exit 0
fi

# Pin to the workspace toolchain (rust-toolchain.toml "1.85") — a bare
# `rustup target add` installs into the DEFAULT toolchain, which may differ.
rustup target add thumbv7em-none-eabihf --toolchain 1.85 >/dev/null 2>&1 || true
echo "==> QEMU runtime smoke: baremetal-qemu"
# Build OUTSIDE the QEMU timeout (a cold build alone can eat the whole 60 s
# budget), with the committed lockfile enforced (--locked) and the tuned
# [profile.release] actually exercised (the debug profile left it dead config).
( cd embedded/baremetal-qemu && cargo build --release --locked )
( cd embedded/baremetal-qemu && timeout 60 cargo run --release --locked )
echo "OK: tst-core muxer byte-matches the video-roundtrip golden under QEMU"
