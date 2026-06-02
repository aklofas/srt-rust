#!/usr/bin/env bash
# Build + run the S3 SRT loopback proof under QEMU mps2-an386, both unencrypted
# and mbedTLS-encrypted, asserting the GOLDEN x N stream recovers byte-exact
# under ~20% injected packet loss (SRT ARQ). Cross-builds libsrt (+ mbedTLS for
# the encrypted phase) on first run. Skips when the ARM C++ cross-toolchain,
# QEMU, or cmake is absent.
set -euo pipefail
cd "$(dirname "$0")/.."

command -v arm-none-eabi-g++ >/dev/null 2>&1 || { echo "SKIP: arm-none-eabi-g++ not installed"; exit 0; }
command -v qemu-system-arm   >/dev/null 2>&1 || { echo "SKIP: qemu-system-arm not installed"; exit 0; }
command -v cmake             >/dev/null 2>&1 || { echo "SKIP: cmake not installed"; exit 0; }

run_phase() {  # $1=label  $2=PASS-token  $3..=env
  local label="$1" token="$2"; shift 2
  echo "==> building S3 ($label)"
  ( cd embedded/s3-srt && env "$@" ./build.sh >/dev/null )
  echo "==> running S3 ($label) under QEMU"
  local out
  out=$(timeout 150 qemu-system-arm -machine mps2-an386 -nographic \
    -semihosting-config enable=on,target=native -kernel embedded/s3-srt/firmware.elf || true)
  echo "$out"
  grep -q "$token" <<<"$out" || { echo "GATE FAILED ($label)"; exit 1; }
}

run_phase "unencrypted" 'PASS: s3_srt_plain' S3_ENCRYPT=0
run_phase "encrypted"   'PASS: s3_srt_aes'   S3_ENCRYPT=1

echo "OK: SRT loopback recovers GOLDEN under loss, unencrypted + mbedTLS-encrypted"
