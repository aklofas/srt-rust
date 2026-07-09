#!/usr/bin/env bash
# Run the freertos-srt NIC-egress example end-to-end: a bare-metal SRT caller
# streams the 564-byte golden x N out a lan9118 NIC over QEMU SLIRP user-net to a
# host tst-srt listener that verifies byte-exact. Two phases: plain + mbedTLS-
# AES-128. The HOST process exit code + PASS token is the verdict (the firmware
# can't self-verify off-device egress; it only prints s4_*_sent). Invoked by
# embedded/scripts/check/freertos-srt.sh example.
# Caller (embedded/scripts/check/freertos-srt.sh) has already verified arm-none-eabi-g++, qemu,
# cmake, and cargo are present.
set -euo pipefail
# This file lives at embedded/scripts/lib/ — three levels under the workspace root.
cd "$(dirname "$0")/../../.."

HOST_DIR=embedded/freertos-srt/example/host
HOST_BIN="$HOST_DIR/target/release/freertos-srt-host"
# Must match build-common.sh's -DSRT_PASSPHRASE for the encrypted firmware.
PASSPHRASE="freertos-srt-secret-1"

run_phase() {  # $1=label  $2=host-PASS-token  $3=ENCRYPT  $4=passphrase-or-empty
  local label="$1" token="$2" enc="$3" pass="${4:-}"
  echo "==> building example firmware ($label)"
  ( cd embedded/freertos-srt && env ENCRYPT="$enc" ./build.sh example >/dev/null )
  echo "==> building example host harness ($label)"
  SRT_FORCE_VENDORED=1 cargo build --manifest-path "$HOST_DIR/Cargo.toml" --release >/dev/null

  echo "==> starting host listener ($label)"
  local hostout; hostout=$(mktemp)
  if [ -n "$pass" ]; then FREERTOS_SRT_PASSPHRASE="$pass" "$HOST_BIN" >"$hostout" 2>&1 &
  else                    "$HOST_BIN" >"$hostout" 2>&1 & fi
  local host_pid=$!

  # Wait (<=10s) for the listener to print 'host-ready' before launching QEMU.
  # Hard-fail if it never appears: otherwise the loop falls through and QEMU
  # connects before the listener is bound -> a race/flaky failure.
  local ready=0
  for _ in $(seq 1 100); do
    if grep -q 'host-ready' <<<"$(cat "$hostout")"; then ready=1; break; fi
    kill -0 "$host_pid" 2>/dev/null || { echo "host died early ($label)"; cat "$hostout"; exit 1; }
    sleep 0.1
  done
  if [ "$ready" -ne 1 ]; then
    echo "GATE FAILED ($label): host never printed host-ready within 10s"; cat "$hostout"
    kill "$host_pid" 2>/dev/null; wait "$host_pid" 2>/dev/null
    exit 1
  fi

  echo "==> running example firmware caller under QEMU ($label)"
  local qout
  qout=$(timeout 120 qemu-system-arm -machine mps2-an386 -nographic \
    -semihosting-config enable=on,target=native \
    -kernel embedded/freertos-srt/build/firmware.elf \
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

# The firmware bakes the host port (example/main.cpp PORT=9000) at compile
# time, so the host listener MUST get 9000. Fail fast and say why, instead of
# a confusing "host died early" when something else already holds the port.
if command -v ss >/dev/null 2>&1 && ss -uln 2>/dev/null | grep -q ':9000 '; then
  echo "GATE FAILED: UDP port 9000 already in use on the host (the firmware's target port is compile-time fixed)"; exit 1
fi

run_phase "plain"     'PASS: s4_host_plain' 0 ''
run_phase "encrypted" 'PASS: s4_host_aes'   1 "$PASSPHRASE"

echo "OK: SRT egress over real lan9118 NIC -> host receiver, plain + AES-128"
