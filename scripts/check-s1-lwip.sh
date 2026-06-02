#!/usr/bin/env bash
# Build + run the S1 FreeRTOS + lwIP harness under QEMU mps2-an386 and assert the
# 564-byte golden round-trips through a lwIP UDP loopback socket. Skips when the
# ARM C++ cross-toolchain or QEMU is absent (same pattern as check-s0-cpp-gate.sh).
set -euo pipefail
cd "$(dirname "$0")/.."

command -v arm-none-eabi-g++ >/dev/null 2>&1 || { echo "SKIP: arm-none-eabi-g++ not installed"; exit 0; }
command -v qemu-system-arm   >/dev/null 2>&1 || { echo "SKIP: qemu-system-arm not installed"; exit 0; }

# golden.h is generated (not committed) from the committed video-roundtrip
# output.ts by check-c-firmware-qemu.sh. Generate it here too if absent so this
# gate is self-contained on a clean checkout (the firmware #includes it).
GOLDEN_H=crates/baremetal-qemu-c/firmware/golden.h
GOLDEN_TS=crates/tst-integration/tests/fixtures/scenarios/video-roundtrip/output.ts
if [ ! -f "$GOLDEN_H" ]; then
  command -v python3 >/dev/null 2>&1 || { echo "SKIP: python3 not installed (needed to generate golden.h)"; exit 0; }
  echo "==> generating $GOLDEN_H from $GOLDEN_TS"
  python3 - "$GOLDEN_TS" > "$GOLDEN_H" <<'PY'
import sys
data = open(sys.argv[1], "rb").read()
print("#include <stddef.h>\n#include <stdint.h>")
print(f"static const size_t GOLDEN_LEN = {len(data)};")
print("static const uint8_t GOLDEN[] = {")
for i in range(0, len(data), 12):
    print("  " + ", ".join(f"0x{b:02x}" for b in data[i:i+12]) + ",")
print("};")
PY
fi

echo "==> building S1 lwIP harness firmware"
( cd embedded/s1-lwip && ./build.sh )

echo "==> running under QEMU (mps2-an386)"
# `|| true` so a non-zero firmware exit (FAIL path) is captured + echoed, not aborted by set -e.
OUT=$(timeout 60 qemu-system-arm -machine mps2-an386 -nographic \
  -semihosting-config enable=on,target=native -kernel embedded/s1-lwip/firmware.elf || true)
echo "$OUT"
# here-string (not `echo | grep -q`) to dodge the known SIGPIPE-under-pipefail flake.
grep -q 'PASS: s1_lwip ' <<<"$OUT" || { echo "GATE FAILED"; exit 1; }
echo "OK: 564B golden round-tripped through lwIP UDP loopback (select + pthreads + hi-res clock)"
