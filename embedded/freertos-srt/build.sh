#!/usr/bin/env bash
# Build one freertos-srt target. Usage: [ENCRYPT=1] ./build.sh <target>
#   targets: exceptions lwip-loopback libsrt-smoke loopback-arq example srt-recv clean
# ENCRYPT (env var, default 0) selects the plain/AES libsrt build for the two
# encryption-capable targets (loopback-arq, example). All output lands under
# build/; `./build.sh clean` removes it.
set -euo pipefail
cd "$(dirname "$0")"
t="${1:?usage: [ENCRYPT=1] ./build.sh <exceptions|lwip-loopback|libsrt-smoke|loopback-arq|loopback-arq-connfail|example|srt-recv|fault-smoke|malloc-stress|clean>}"
if [ "$t" = "clean" ]; then rm -rf build; echo "removed build/"; exit 0; fi
export ENCRYPT="${ENCRYPT:-0}" DEFS="" TSTC=0
case "$t" in
  exceptions)    export LWIP=0 LIBSRT=0 NETIF=none    APP="tests/exceptions/main.cpp" ;;
  lwip-loopback) export LWIP=1 LIBSRT=0 NETIF=none    APP="tests/lwip-loopback/main.cpp"
                 export DEFS="-DLWIP_NETIF_LOOPBACK=1 -DLWIP_HAVE_LOOPIF=1 -DLWIP_NETIF_LOOPBACK_MULTITHREADING=1" ;;
  libsrt-smoke)  export LWIP=1 LIBSRT=1 NETIF=none    APP="tests/libsrt-smoke/main.cpp" ;;
  loopback-arq)  export LWIP=1 LIBSRT=1 NETIF=lossy   APP="tests/loopback-arq/main.cpp" DEFS="-DSRT_FILE_MODE -DFREERTOS_SRT_LOSS_ENABLED=1" ;;
  loopback-arq-connfail) export LWIP=1 LIBSRT=1 NETIF=lossy APP="tests/loopback-arq/main.cpp" DEFS="-DSRT_FILE_MODE -DFREERTOS_SRT_LOSS_ENABLED=1 -DFREERTOS_SRT_CONNECT_PORT=9002" ;;
  example)       export LWIP=1 LIBSRT=1 NETIF=lan9118 APP="example/main.cpp" ;;
  srt-recv)      export LWIP=1 LIBSRT=1 NETIF=lan9118 APP="srt-recv/main.cpp" TSTC=1 ;;
  fault-smoke)   export LWIP=0 LIBSRT=0 NETIF=none    APP="tests/fault-smoke/main.cpp" ;;
  malloc-stress) export LWIP=0 LIBSRT=0 NETIF=none    APP="tests/malloc-stress/main.cpp" ;;
  *) echo "unknown target: $t" >&2; exit 2 ;;
esac
# TST_QEMU_TEST_ENTROPY gates syscalls_stub.c's deterministic entropy hooks
# (see its header comment) — every target built by this script is a QEMU test
# build, so it's appended here unconditionally, after the per-target DEFS
# above (which some cases overwrite rather than extend).
export DEFS="${DEFS:-} -DTST_QEMU_TEST_ENTROPY=1"
source substrate/build-common.sh
