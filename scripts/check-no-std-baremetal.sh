#!/usr/bin/env bash
# Local mirror of the CI `no-std-baremetal` job.
#
# Proves `tst-core` compiles `#![no_std]` + `alloc` for bare-metal targets.
# A `*-none-*` target has no `std` at all, so a clean build IS the guard:
# any `use std::` regression or std-only dependency fails to compile here.
# Library builds need no `#[global_allocator]` (only final binaries do), so
# this is a pure compile gate.
#
#   thumbv7em-none-eabihf        = Cortex-M4F / M7F (STM32F4 / F7 / H7)
#   riscv32imac-unknown-none-elf = bare-metal RISC-V (e.g. ESP32-P4 without esp-idf)
set -euo pipefail
cd "$(dirname "$0")/.."

TARGETS=(thumbv7em-none-eabihf riscv32imac-unknown-none-elf)
for t in "${TARGETS[@]}"; do
  rustup target add "$t" >/dev/null 2>&1 || true
  echo "==> cargo build -p tst-core --no-default-features --target $t"
  cargo build -p tst-core --no-default-features --target "$t"
done
echo "OK: tst-core builds no_std for ${TARGETS[*]}"
