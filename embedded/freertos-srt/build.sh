#!/usr/bin/env bash
# Build one freertos-srt target. Usage: ./build.sh <target> [ENCRYPT=0|1]
#   targets: exceptions lwip-loopback libsrt-smoke loopback-arq example
# ENCRYPT (env, default 0) selects the plain/AES libsrt build for the two
# encryption-capable targets (loopback-arq, example).
set -euo pipefail
cd "$(dirname "$0")"
t="${1:?usage: build.sh <exceptions|lwip-loopback|libsrt-smoke|loopback-arq|example>}"
export ENCRYPT="${ENCRYPT:-0}" DEFS=""
case "$t" in
  exceptions)    export LWIP=0 LIBSRT=0 NETIF=none    APP="tests/exceptions/main.cpp" ;;
  lwip-loopback) export LWIP=1 LIBSRT=0 NETIF=none    APP="tests/lwip-loopback/main.cpp"
                 export DEFS="-DLWIP_NETIF_LOOPBACK=1 -DLWIP_HAVE_LOOPIF=1 -DLWIP_NETIF_LOOPBACK_MULTITHREADING=1" ;;
  libsrt-smoke)  export LWIP=1 LIBSRT=1 NETIF=none    APP="tests/libsrt-smoke/main.cpp" ;;
  loopback-arq)  export LWIP=1 LIBSRT=1 NETIF=lossy   APP="tests/loopback-arq/main.cpp" DEFS="-DSRT_FILE_MODE -DS3_LOSS_ENABLED=1" ;;
  example)       export LWIP=1 LIBSRT=1 NETIF=lan9118 APP="example/main.cpp" ;;
  *) echo "unknown target: $t" >&2; exit 2 ;;
esac
source substrate/build-common.sh
