#!/usr/bin/env bash
# Local mirror of the CI QEMU runtime gate.
#
# Runs the tst-core muxer on a Cortex-M4 under QEMU and asserts it byte-matches
# the committed video-roundtrip golden (see embedded/baremetal-qemu/). Skips
# gracefully when qemu-system-arm is not installed, so pre-push runs on
# machines without QEMU do not fail — same optional-tool pattern as the
# ffmpeg-dependent tests.
set -euo pipefail
cd "$(dirname "$0")/.."

if ! command -v qemu-system-arm >/dev/null 2>&1; then
  echo "SKIP: qemu-system-arm not installed (apt install qemu-system-arm)"
  exit 0
fi

# Pin the target to the workspace toolchain (rust-toolchain.toml channel "1.85").
# A bare `rustup target add` installs into the DEFAULT toolchain, which may not
# be the one `cargo run` resolves via rust-toolchain.toml — so pin explicitly.
rustup target add thumbv7em-none-eabihf --toolchain 1.85 >/dev/null 2>&1 || true
echo "==> QEMU runtime smoke: baremetal-qemu"
( cd embedded/baremetal-qemu && timeout 60 cargo run )
echo "OK: tst-core muxer byte-matches the video-roundtrip golden under QEMU"
