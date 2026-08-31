#!/usr/bin/env bash
# Run the freertos-srt NIC-ingress srt-recv gate end-to-end: a bare-metal SRT
# LISTENER on a real lan9118 NIC accepts an inbound connection from a HOST
# tst-srt CALLER (this gate's own host driver, `--send` mode) over QEMU SLIRP
# user-net (hostfwd=udp forwards the port), receives the 564-byte golden, and
# demuxes + verifies it ON-DEVICE via the offline tst_demuxer_* C ABI. This is
# the reverse of run-freertos-srt-example.sh in both data direction AND
# verifier role: there the firmware streams and the HOST verifies; here the
# host streams and the FIRMWARE verifies — so the verdict lives in QEMU's own
# PASS token, and the host driver only needs to prove the bytes were sent.
# Invoked by embedded/scripts/check/freertos-srt.sh srt-recv.
# Caller (embedded/scripts/check/freertos-srt.sh) has already verified
# arm-none-eabi-g++, qemu, cmake, and cargo are present.
set -euo pipefail
# This file lives at embedded/scripts/lib/ — three levels under the workspace root.
cd "$(dirname "$0")/../../.."

HOST_DIR=embedded/freertos-srt/example/host
HOST_BIN="$HOST_DIR/target/release/freertos-srt-host"
# GUEST_PORT is compile-time fixed in srt-recv/main.cpp (PORT). HOST_PORT is
# the SLIRP-side redirect the host driver actually connects to; kept distinct
# from GUEST_PORT and from example's 9000 / loopback-arq's 9001 so a stray
# leftover listener from another leg can't collide.
GUEST_PORT=9003
HOST_PORT=19003

echo "==> building srt-recv firmware"
( cd embedded/freertos-srt && ENCRYPT=0 ./build.sh srt-recv >/dev/null )
echo "==> building host harness (shared example/host crate, --send mode)"
SRT_FORCE_VENDORED=1 cargo build --manifest-path "$HOST_DIR/Cargo.toml" --release --locked >/dev/null

if command -v ss >/dev/null 2>&1 && ss -uln 2>/dev/null | grep -q ":$HOST_PORT "; then
  echo "GATE FAILED: UDP port $HOST_PORT already in use on the host (the srt-recv redirect target)"; exit 1
fi

echo "==> booting QEMU (srt-recv firmware, hostfwd udp $HOST_PORT -> guest $GUEST_PORT)"
QOUT=$(mktemp)
timeout 60 qemu-system-arm -machine mps2-an386 -nographic \
  -semihosting-config enable=on,target=native \
  -kernel embedded/freertos-srt/build/firmware.elf \
  -nic user,model=lan9118,hostfwd=udp::$HOST_PORT-:$GUEST_PORT \
  > "$QOUT" 2>&1 &
QPID=$!

# Wait (<=20s) for the firmware to print 'guest-ready' (bind+listen done)
# before launching the host sender -- otherwise the sender's connect could
# race the firmware's boot (lwIP init + srt_startup + bind/listen), a
# multi-second bring-up under QEMU/TCG emulation. If QEMU exits/dies first
# (a boot-time fault), stop waiting immediately rather than spinning to 20s.
ready=0
for _ in $(seq 1 200); do
  if grep -q 'guest-ready' "$QOUT" 2>/dev/null; then ready=1; break; fi
  kill -0 "$QPID" 2>/dev/null || break
  sleep 0.1
done
if [ "$ready" -ne 1 ]; then
  echo "GATE FAILED: firmware never printed guest-ready within 20s (or QEMU exited early)"
  echo "----- QEMU output -----"; cat "$QOUT"
  kill "$QPID" 2>/dev/null; wait "$QPID" 2>/dev/null || true
  rm -f "$QOUT"
  exit 1
fi

echo "==> running host sender (--send)"
SENDOUT=$(mktemp)
send_rc=0
timeout 30 "$HOST_BIN" --send "127.0.0.1:$HOST_PORT" > "$SENDOUT" 2>&1 || send_rc=$?
cat "$SENDOUT"

# Bounded join: the sender has already returned (or been killed by the 30s
# timeout above), so QEMU should be finishing up (on-device demux is fast).
# Its own `timeout 60` wrapper is the hard backstop against an unresponsive
# guest wedging the gate; wait for it to actually exit.
qemu_rc=0
wait "$QPID" 2>/dev/null || qemu_rc=$?
echo "----- QEMU output -----"; cat "$QOUT"

fail=0
if [ "$send_rc" -ne 0 ]; then
  echo "GATE FAILED: host sender exited $send_rc"; fail=1
fi
if ! grep -q 'PASS: srt_recv ' "$QOUT"; then
  echo "GATE FAILED: no firmware PASS token (qemu rc=$qemu_rc)"
  echo "  (rc=124 + full budget = hang/timeout; fast nonzero rc = a labeled FAIL[...] exit)"
  fail=1
fi
rm -f "$QOUT" "$SENDOUT"
[ "$fail" -eq 0 ] || exit 1

echo "OK: SRT ingress over real lan9118 NIC <- host sender, on-device demux + byte-exact verify"
