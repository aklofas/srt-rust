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

  # Join the host (bounded), then assert ITS verdict.
  local hrc=0; { wait "$host_pid"; } || hrc=$?
  echo "---- host output ($label) ----"; cat "$hostout"
  grep -q "$token" <<<"$(cat "$hostout")" || { echo "GATE FAILED ($label): no host PASS"; exit 1; }
  [ "$hrc" -eq 0 ] || { echo "GATE FAILED ($label): host exit $hrc"; exit 1; }
  rm -f "$hostout"
}

run_phase "plain"     'PASS: s4_host_plain' 0 ''
run_phase "encrypted" 'PASS: s4_host_aes'   1 's4-egress-secret-1'

echo "OK: SRT egress over real lan9118 NIC -> host receiver, plain + AES-128"
