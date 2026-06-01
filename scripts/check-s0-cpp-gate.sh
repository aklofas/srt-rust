#!/usr/bin/env bash
# Build + run the S0 C++ exceptions/threading gate under QEMU mps2-an386 and
# assert per-task exception isolation. Skips when the ARM C++ cross-toolchain or
# QEMU is absent (same pattern as check-c-firmware-qemu.sh).
set -euo pipefail
cd "$(dirname "$0")/.."

command -v arm-none-eabi-g++ >/dev/null 2>&1 || { echo "SKIP: arm-none-eabi-g++ not installed"; exit 0; }
command -v qemu-system-arm   >/dev/null 2>&1 || { echo "SKIP: qemu-system-arm not installed"; exit 0; }

echo "==> building S0 gate firmware"
( cd embedded/s0-cpp-gate && ./build.sh )

echo "==> running under QEMU (mps2-an386)"
# `|| true` so a non-zero firmware exit (FAIL path) is captured + echoed, not aborted by set -e.
OUT=$(timeout 60 qemu-system-arm -machine mps2-an386 -nographic \
  -semihosting-config enable=on,target=native -kernel embedded/s0-cpp-gate/firmware.elf || true)
echo "$OUT"
# here-string (not `echo | grep -q`) to dodge the known SIGPIPE-under-pipefail flake.
grep -q 'PASS: s0_cpp_gate' <<<"$OUT" || { echo "GATE FAILED"; exit 1; }
echo "OK: concurrent FreeRTOS tasks throw/catch with per-task exception isolation"
