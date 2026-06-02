#!/usr/bin/env bash
# Run the freertos-srt NIC-egress example end-to-end: a bare-metal SRT caller
# streams the 564-byte golden x N out a lan9118 NIC over QEMU SLIRP user-net to a
# host tst-srt listener that verifies byte-exact. Two phases: plain + mbedTLS-
# AES-128. The HOST process exit code + PASS token is the verdict (the firmware
# can't self-verify off-device egress; it only prints s4_*_sent). Lifted from the
# former scripts/check-s4-srt.sh; invoked by check-freertos-srt.sh example.
# Caller (check-freertos-srt.sh) has already verified arm-none-eabi-g++, qemu,
# cmake, and cargo are present.
set -euo pipefail
cd "$(dirname "$0")/../.."

HOST_DIR=embedded/freertos-srt/example/host
HOST_BIN="$HOST_DIR/target/release/s4-host"
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
  if [ -n "$pass" ]; then S4_PASSPHRASE="$pass" "$HOST_BIN" >"$hostout" 2>&1 &
  else                    "$HOST_BIN" >"$hostout" 2>&1 & fi
  local host_pid=$!

  # Wait (<=10s) for the listener to print 'host-ready' before launching QEMU.
  for _ in $(seq 1 100); do
    grep -q 'host-ready' <<<"$(cat "$hostout")" && break
    kill -0 "$host_pid" 2>/dev/null || { echo "host died early"; cat "$hostout"; exit 1; }
    sleep 0.1
  done

  echo "==> running example firmware caller under QEMU ($label)"
  local qout
  qout=$(timeout 120 qemu-system-arm -machine mps2-an386 -nographic \
    -semihosting-config enable=on,target=native \
    -kernel embedded/freertos-srt/firmware.elf \
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
run_phase "encrypted" 'PASS: s4_host_aes'   1 "$PASSPHRASE"

echo "OK: SRT egress over real lan9118 NIC -> host receiver, plain + AES-128"
