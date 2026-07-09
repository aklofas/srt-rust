#!/usr/bin/env bash
# Generate a C header embedding a binary golden as GOLDEN[] / GOLDEN_LEN.
# Usage: gen-golden-h.sh <input-binary> <output-header>
# Shared by freertos-srt's build-common.sh and firmware-qemu.sh so the emitter
# exists exactly once. Atomic: writes a temp file and renames into place, so an
# interrupted or failed run can never leave a truncated/empty header for a
# later run to trust. Callers regenerate unconditionally — the golden is 564
# bytes, this costs ~50 ms — which also makes a fixture change take effect
# immediately (the old [ ! -f ] guard kept a stale header forever).
set -euo pipefail
in="${1:?usage: gen-golden-h.sh <input-binary> <output-header>}"
out="${2:?usage: gen-golden-h.sh <input-binary> <output-header>}"
tmp="$out.tmp.$$"
trap 'rm -f "$tmp"' EXIT
python3 - "$in" > "$tmp" <<'PY'
import sys
data = open(sys.argv[1], "rb").read()
print("#include <stddef.h>\n#include <stdint.h>")
print(f"static const size_t GOLDEN_LEN = {len(data)};")
print("static const uint8_t GOLDEN[] = {")
for i in range(0, len(data), 12):
    print("  " + ", ".join(f"0x{b:02x}" for b in data[i:i+12]) + ",")
print("};")
PY
mv "$tmp" "$out"
