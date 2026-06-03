#!/usr/bin/env bash
# Build + run one freertos-srt target under QEMU mps2-an386 and assert its PASS
# token(s). Usage: scripts/check/embedded/freertos-srt.sh <target>.
#
# Missing prerequisites (ARM cross-toolchain / QEMU / cmake / cargo) skip cleanly
# by default so a local box without the toolchain is green. Set
# FREERTOS_SRT_REQUIRE_TOOLS=1 to fail closed instead — CI hard-gates set it so a
# runner-image regression that drops a prerequisite goes red rather than quietly
# reducing coverage to a no-op pass.
set -euo pipefail
cd "$(dirname "$0")/../../.."
t="${1:?usage: scripts/check/embedded/freertos-srt.sh <exceptions|lwip-loopback|libsrt-smoke|loopback-arq|example>}"
D=embedded/freertos-srt
REQUIRE="${FREERTOS_SRT_REQUIRE_TOOLS:-0}"

# Skip (default) or fail closed (FREERTOS_SRT_REQUIRE_TOOLS=1) when a tool is
# absent. A skip exits 0; a required-but-missing tool exits 1.
need() { # $1=binary  $2=human label
  command -v "$1" >/dev/null 2>&1 && return 0
  if [ "$REQUIRE" = "1" ]; then echo "FATAL: required tool '$2' not installed (FREERTOS_SRT_REQUIRE_TOOLS=1)"; exit 1; fi
  echo "SKIP: $2 not installed"; exit 0
}
need arm-none-eabi-g++ arm-none-eabi-g++
need qemu-system-arm   qemu-system-arm

# Run firmware.elf under QEMU and assert a PASS token. On failure the full QEMU
# transcript is printed (CI logs would otherwise show only "GATE FAILED" and
# drop the firmware's FAIL[...] line). Token-only — not exit-code — because ARM
# semihosting SYS_EXIT propagation through qemu-system-arm is version-dependent;
# the firmware prints exactly one PASS line on success, so the token is the
# authoritative verdict.
assert_pass() { # $1=timeout  $2=token  $3=label
  local out
  out=$(timeout "$1" qemu-system-arm -machine mps2-an386 -nographic \
        -semihosting-config enable=on,target=native -kernel "$D/build/firmware.elf" || true)
  grep -q "$2" <<<"$out" && return 0
  echo "GATE FAILED ($3)"; echo "----- QEMU output ($3) -----"; echo "$out"; exit 1
}

case "$t" in
  exceptions)    ( cd "$D" && ./build.sh exceptions >/dev/null )
                 assert_pass 60 'PASS: s0_cpp_gate' "$t" ;;
  lwip-loopback) ( cd "$D" && ./build.sh lwip-loopback >/dev/null )
                 assert_pass 60 'PASS: s1_lwip ' "$t" ;;
  libsrt-smoke)  need cmake cmake
                 ( cd "$D" && ./build.sh libsrt-smoke >/dev/null )
                 assert_pass 90 'PASS: s2_libsrt' "$t" ;;
  loopback-arq)  need cmake cmake
                 ( cd "$D" && ENCRYPT=0 ./build.sh loopback-arq >/dev/null )
                 assert_pass 150 'PASS: s3_srt_plain' "$t plain"
                 ( cd "$D" && ENCRYPT=1 ./build.sh loopback-arq >/dev/null )
                 assert_pass 150 'PASS: s3_srt_aes' "$t aes" ;;
  example)       need cmake cmake
                 need cargo cargo
                 bash "$(dirname "$0")/../../lib/run-freertos-srt-example.sh" || exit 1 ;;
  *)             echo "unknown target: $t (expected exceptions|lwip-loopback|libsrt-smoke|loopback-arq|example)" >&2; exit 2 ;;
esac
echo "OK: freertos-srt $t"
