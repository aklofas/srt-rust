#!/usr/bin/env bash
# Build + run the S2 libsrt boot smoke under QEMU mps2-an386 and assert
# srt_startup+create_socket+getsockstate+close+cleanup all succeed on the
# FreeRTOS+lwIP substrate. Cross-builds libsrt.a on first run (cmake). Skips
# when the ARM C++ cross-toolchain, QEMU, or cmake is absent.
set -euo pipefail
cd "$(dirname "$0")/.."

command -v arm-none-eabi-g++ >/dev/null 2>&1 || { echo "SKIP: arm-none-eabi-g++ not installed"; exit 0; }
command -v qemu-system-arm   >/dev/null 2>&1 || { echo "SKIP: qemu-system-arm not installed"; exit 0; }
command -v cmake             >/dev/null 2>&1 || { echo "SKIP: cmake not installed"; exit 0; }

echo "==> building S2 libsrt firmware (cross-builds libsrt.a on first run)"
( cd embedded/s2-libsrt && ./build.sh )

echo "==> running under QEMU (mps2-an386)"
OUT=$(timeout 120 qemu-system-arm -machine mps2-an386 -nographic \
  -semihosting-config enable=on,target=native -kernel embedded/s2-libsrt/firmware.elf || true)
echo "$OUT"
grep -q 'PASS: s2_libsrt' <<<"$OUT" || { echo "GATE FAILED"; exit 1; }
echo "OK: cross-compiled libsrt initializes on FreeRTOS+lwIP (startup+socket+cleanup)"
