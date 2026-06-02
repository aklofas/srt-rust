#!/usr/bin/env bash
# Build + run one freertos-srt target under QEMU mps2-an386 and assert its PASS
# token(s). Usage: check-freertos-srt.sh <target>. Skips when the ARM cross-
# toolchain / QEMU / cmake / cargo is absent.
set -euo pipefail
cd "$(dirname "$0")/.."
t="${1:?usage: check-freertos-srt.sh <exceptions|lwip-loopback|libsrt-smoke|loopback-arq|example>}"
command -v arm-none-eabi-g++ >/dev/null 2>&1 || { echo "SKIP: arm-none-eabi-g++ not installed"; exit 0; }
command -v qemu-system-arm   >/dev/null 2>&1 || { echo "SKIP: qemu-system-arm not installed"; exit 0; }
D=embedded/freertos-srt

qemu() { timeout "${1:-120}" qemu-system-arm -machine mps2-an386 -nographic \
  -semihosting-config enable=on,target=native -kernel "$D/firmware.elf" "${@:2}" || true; }

case "$t" in
  exceptions)    ( cd "$D" && ./build.sh exceptions >/dev/null )
                 grep -q 'PASS: s0_cpp_gate' <<<"$(qemu 60)" || { echo "GATE FAILED ($t)"; exit 1; } ;;
  lwip-loopback) ( cd "$D" && ./build.sh lwip-loopback >/dev/null )
                 grep -q 'PASS: s1_lwip ' <<<"$(qemu 60)" || { echo "GATE FAILED ($t)"; exit 1; } ;;
  libsrt-smoke)  command -v cmake >/dev/null 2>&1 || { echo "SKIP: cmake"; exit 0; }
                 ( cd "$D" && ./build.sh libsrt-smoke >/dev/null )
                 grep -q 'PASS: s2_libsrt' <<<"$(qemu 90)" || { echo "GATE FAILED ($t)"; exit 1; } ;;
  loopback-arq)  command -v cmake >/dev/null 2>&1 || { echo "SKIP: cmake"; exit 0; }
                 ( cd "$D" && ENCRYPT=0 ./build.sh loopback-arq >/dev/null )
                 grep -q 'PASS: s3_srt_plain' <<<"$(qemu 150)" || { echo "GATE FAILED ($t plain)"; exit 1; }
                 ( cd "$D" && ENCRYPT=1 ./build.sh loopback-arq >/dev/null )
                 grep -q 'PASS: s3_srt_aes' <<<"$(qemu 150)" || { echo "GATE FAILED ($t aes)"; exit 1; } ;;
  example)       command -v cmake >/dev/null 2>&1 || { echo "SKIP: cmake"; exit 0; }
                 command -v cargo >/dev/null 2>&1 || { echo "SKIP: cargo"; exit 0; }
                 bash "$(dirname "$0")/lib/run-freertos-srt-example.sh" || exit 1 ;;
  *)             echo "unknown target: $t (expected exceptions|lwip-loopback|libsrt-smoke|loopback-arq|example)" >&2; exit 2 ;;
esac
echo "OK: freertos-srt $t"
