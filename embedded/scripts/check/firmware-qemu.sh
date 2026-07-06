#!/usr/bin/env bash
# Build the C firmware (links libtstrans_firmware.a = tst-c-core's offline C ABI
# built no_std) and run it under QEMU mps2-an386, asserting it byte-matches the
# committed video-roundtrip golden through the C ABI. Skips when the ARM C
# cross-toolchain or QEMU is absent (same optional-tool pattern as
# embedded/scripts/check/qemu-runtime.sh).
set -euo pipefail
cd "$(dirname "$0")/../../.."

if ! command -v arm-none-eabi-gcc >/dev/null 2>&1; then
  echo "SKIP: arm-none-eabi-gcc not installed (apt install gcc-arm-none-eabi libnewlib-arm-none-eabi)"; exit 0
fi
if ! command -v qemu-system-arm >/dev/null 2>&1; then
  echo "SKIP: qemu-system-arm not installed (apt install qemu-system-arm)"; exit 0
fi

CRATE=embedded/baremetal-qemu-c
FW="$CRATE/firmware"
GOLDEN_TS=crates/tst-integration/tests/fixtures/scenarios/video-roundtrip/output.ts

rustup target add thumbv7em-none-eabihf --toolchain 1.85 >/dev/null 2>&1 || true

echo "==> generating golden.h from $GOLDEN_TS"
python3 - "$GOLDEN_TS" > "$FW/golden.h" <<'PY'
import sys
data = open(sys.argv[1], "rb").read()
print("#include <stddef.h>\n#include <stdint.h>")
print(f"static const size_t GOLDEN_LEN = {len(data)};")
print("static const uint8_t GOLDEN[] = {")
for i in range(0, len(data), 12):
    print("  " + ", ".join(f"0x{b:02x}" for b in data[i:i+12]) + ",")
print("};")
PY

echo "==> building glue staticlib (thumbv7em)"
( cd "$CRATE" && cargo build --release )
# Deterministic path: --release above + target pinned to thumbv7em-none-eabihf
# in the crate's .cargo/config.toml. (A `find | head -1` would walk debug/
# first and could link a stale debug archive from a prior `cargo build`.)
AR="$CRATE/target/thumbv7em-none-eabihf/release/libtstrans_firmware.a"
ARDIR=$(cd "$(dirname "$AR")" && pwd)
INC=$(cd bindings/c/include && pwd)

echo "==> compiling + linking firmware.elf"
( cd "$FW" && arm-none-eabi-gcc \
    -mcpu=cortex-m4 -mthumb -mfloat-abi=hard -mfpu=fpv4-sp-d16 \
    -Os -ffunction-sections -fdata-sections --specs=nano.specs --specs=rdimon.specs \
    -T mps2_an386.ld -Wl,--gc-sections -DTST_SKIP_ABI_ASSERTS -I "$INC" \
    startup.c main.c \
    -Wl,--start-group "-L$ARDIR" -ltstrans_firmware -lc -lm -lrdimon -Wl,--end-group \
    -o firmware.elf )

echo "==> running firmware under QEMU (mps2-an386)"
# `|| true` so a non-zero firmware exit (FAIL path / semihosting _exit(1)) is
# captured here instead of aborting via `set -e` before the diagnostic is
# echoed — CI logs need the firmware's FAIL line to triage a regression.
OUT=$(timeout 60 qemu-system-arm -machine mps2-an386 -nographic \
  -semihosting-config enable=on,target=native -kernel "$FW/firmware.elf" || true)
echo "$OUT"
echo "$OUT" | grep -q 'PASS: c_firmware' || { echo "GATE FAILED"; exit 1; }
echo "OK: C firmware muxer byte-matches the video-roundtrip golden under QEMU"
