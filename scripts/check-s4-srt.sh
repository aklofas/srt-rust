#!/usr/bin/env bash
# Build + run the S4 real-NIC egress proof: a bare-metal SRT caller streams the
# 564-byte golden x N out a lan9118 NIC over QEMU SLIRP user-net to a host
# tst-srt listener that verifies byte-exact. Two phases: plain + mbedTLS-AES-128.
# The HOST process exit code + PASS token is the verdict. Skips when the ARM C++
# cross-toolchain, QEMU, cmake, or cargo is absent.
set -euo pipefail
cd "$(dirname "$0")/.."

command -v arm-none-eabi-g++ >/dev/null 2>&1 || { echo "SKIP: arm-none-eabi-g++ not installed"; exit 0; }
command -v qemu-system-arm   >/dev/null 2>&1 || { echo "SKIP: qemu-system-arm not installed"; exit 0; }
command -v cmake             >/dev/null 2>&1 || { echo "SKIP: cmake not installed"; exit 0; }
command -v cargo             >/dev/null 2>&1 || { echo "SKIP: cargo not installed"; exit 0; }

# golden.h is generated (not committed) from the committed video-roundtrip
# output.ts by check-c-firmware-qemu.sh. Generate it here too if absent so this
# gate is self-contained on a clean checkout — both main.cpp (#include) and the
# host build.rs depend on the same 564 golden bytes.
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

HOST_DIR=embedded/s4-srt/host
HOST_BIN="$HOST_DIR/target/release/s4-host"

run_phase() {  # $1=label  $2=host-PASS-token  $3=S4_ENCRYPT  $4=passphrase-or-empty
  local label="$1" token="$2" enc="$3" pass="${4:-}"
  echo "==> building S4 firmware ($label)"
  ( cd embedded/s4-srt && env S4_ENCRYPT="$enc" ./build.sh >/dev/null )
  echo "==> building S4 host harness ($label)"
  SRT_FORCE_VENDORED=1 cargo build --manifest-path "$HOST_DIR/Cargo.toml" --release >/dev/null

  echo "==> starting host listener ($label)"
  local hostout; hostout=$(mktemp)
  if [ -n "$pass" ]; then S4_PASSPHRASE="$pass" "$HOST_BIN" >"$hostout" 2>&1 &
  else                    "$HOST_BIN" >"$hostout" 2>&1 & fi
  local host_pid=$!

  # Wait (<=10s) for the listener to print 'host-ready' before launching QEMU.
  for _ in $(seq 1 100); do
    grep -q 'host-ready' <<<"$(cat "$hostout")" && break
    kill -0 "$host_pid" 2>/dev/null || { echo "host died early"; cat "$hostout"; exit 1; }
    sleep 0.1
  done

  echo "==> running S4 firmware caller under QEMU ($label)"
  local qout
  qout=$(timeout 120 qemu-system-arm -machine mps2-an386 -nographic \
    -semihosting-config enable=on,target=native \
    -kernel embedded/s4-srt/firmware.elf \
    -nic user,model=lan9118 || true)
  echo "$qout"

  # Bounded join: once QEMU (the caller) has exited, the host either already
  # finished (success: it received the stream and exit(0)'d) or is still blocked
  # in accept()/recv() (failure: handshake never connected). Wait up to 10s,
  # then kill it so a failed run can't hang CI on an unbounded `wait`.
  local hrc=0 waited=0
  while kill -0 "$host_pid" 2>/dev/null && [ "$waited" -lt 100 ]; do
    sleep 0.1; waited=$((waited + 1))
  done
  if kill -0 "$host_pid" 2>/dev/null; then
    echo "host still running after QEMU exit — killing ($label)"
    kill "$host_pid" 2>/dev/null; wait "$host_pid" 2>/dev/null; hrc=1
  else
    wait "$host_pid" 2>/dev/null || hrc=$?
  fi
  echo "---- host output ($label) ----"; cat "$hostout"
  grep -q "$token" <<<"$(cat "$hostout")" || { echo "GATE FAILED ($label): no host PASS"; exit 1; }
  [ "$hrc" -eq 0 ] || { echo "GATE FAILED ($label): host exit $hrc"; exit 1; }
  rm -f "$hostout"
}

run_phase "plain"     'PASS: s4_host_plain' 0 ''
run_phase "encrypted" 'PASS: s4_host_aes'   1 's4-egress-secret-1'

echo "OK: SRT egress over real lan9118 NIC -> host receiver, plain + AES-128"
