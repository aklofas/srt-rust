#!/usr/bin/env bash
# Parameterized firmware build for the freertos-srt product. Sourced by the
# top-level build.sh dispatcher, which sets the per-target knobs:
#   LWIP=0|1  LIBSRT=0|1  NETIF=none|lossy|lan9118  ENCRYPT=0|1
#   APP="<app .cpp ...>"  DEFS="<extra -D...>"
# Build artifacts (*.o, firmware.elf, srt-*/, mbedtls-*) land in the product
# root (embedded/freertos-srt/) and are gitignored.
set -euo pipefail
PROD=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)   # embedded/freertos-srt
SUB="$PROD/substrate"
ROOT=$(cd "$PROD/../.." && pwd)                          # workspace root
cd "$PROD"

K=$ROOT/vendor/freertos-kernel
P=$ROOT/vendor/freertos-plus-posix
PS=$P/FreeRTOS-Plus-POSIX/source
L=$ROOT/vendor/lwip
SRT=$ROOT/vendor/srt
MBED=$ROOT/vendor/mbedtls
CC=arm-none-eabi-gcc; CXX=arm-none-eabi-g++

ARCH="-mcpu=cortex-m4 -mthumb -mfloat-abi=hard -mfpu=fpv4-sp-d16"
OPT="-Os -ffunction-sections -fdata-sections -g"

# Generate golden.h (564B video-roundtrip) into substrate/ if absent; targets
# that #include it get -I$SUB. Self-contained on a clean checkout.
GOLDEN_TS=$ROOT/crates/tst-integration/tests/fixtures/scenarios/video-roundtrip/output.ts
if [ ! -f "$SUB/golden.h" ]; then
  python3 - "$GOLDEN_TS" > "$SUB/golden.h" <<'PY'
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

# Base includes: substrate roots (lwipopts.h, FreeRTOSConfig.h, arch/, golden.h,
# srt_opts.h, drivers) + FreeRTOS + FreeRTOS-Plus-POSIX. posix-shims + lwIP +
# srt includes are appended only for the layers that need them.
INC="-I$SUB -I$SUB/freertos -I$SUB/lwip -I$SUB/drivers \
     -I$K/include -I$K/portable/GCC/ARM_CM4F \
     -I$P/include -I$P/include/private \
     -I$P/FreeRTOS-Plus-POSIX/include -I$P/FreeRTOS-Plus-POSIX/include/portable"

# Substrate C always present: startup, clock, stubs, FreeRTOS kernel + POSIX.
# (net_shim.c is the libsrt-on-lwIP __wrap_setsockopt shim — it needs lwIP
# headers and is only meaningful on the data-plane targets, so it rides with the
# netif drivers below, exactly as in S3/S4. The non-netif targets never call
# setsockopt, so -Wl,--wrap=setsockopt stays inert for them.)
C_SRC="$SUB/startup.c $SUB/clock_shim.c $SUB/syscalls_stub.c $SUB/atomic64_stub.c \
       $K/tasks.c $K/list.c $K/queue.c $K/timers.c $K/event_groups.c \
       $K/portable/GCC/ARM_CM4F/port.c $K/portable/MemMang/heap_4.c \
       $PS/FreeRTOS_POSIX_pthread.c $PS/FreeRTOS_POSIX_pthread_mutex.c \
       $PS/FreeRTOS_POSIX_pthread_cond.c $PS/FreeRTOS_POSIX_pthread_barrier.c \
       $PS/FreeRTOS_POSIX_clock.c $PS/FreeRTOS_POSIX_utils.c \
       $PS/FreeRTOS_POSIX_sched.c $PS/FreeRTOS_POSIX_unistd.c \
       $PS/FreeRTOS_POSIX_timer.c $PS/FreeRTOS_POSIX_semaphore.c"

if [ "${LWIP:-0}" = "1" ]; then
  INC="$INC -I$L/src/include -I$L/src/include/compat/posix"
  C_SRC="$C_SRC $SUB/lwip/sys_arch.c $L/src/core/*.c $L/src/core/ipv4/*.c $L/src/core/ipv6/*.c $L/src/api/*.c $L/src/netif/ethernet.c"
fi
case "${NETIF:-none}" in
  lossy)   C_SRC="$C_SRC $SUB/drivers/lossy_netif.c $SUB/net_shim.c" ;;
  lan9118) C_SRC="$C_SRC $SUB/drivers/lan9118_netif.c $SUB/net_shim.c" ;;
esac

# libsrt (+ optional mbedTLS) cross-build, keyed on ENCRYPT so plain/AES trees
# don't clobber. Only for LIBSRT targets.
SRT_LIB=""; MBED_LIBS=""; MBED_INSTALL="$PROD/mbedtls-install"
if [ "${LIBSRT:-0}" = "1" ]; then
  SRT_INSTALL="$PROD/srt-install${ENCRYPT:-0}"; SRT_BUILD="$PROD/srt-build${ENCRYPT:-0}"
  SRT_ENC_FLAGS="-DENABLE_ENCRYPTION=OFF"
  if [ "${ENCRYPT:-0}" = "1" ]; then
    if [ ! -f "$MBED_INSTALL/lib/libmbedcrypto.a" ] || [ "${REBUILD:-0}" = "1" ]; then
      rm -rf "$PROD/mbedtls-build" "$MBED_INSTALL"
      cmake -S "$MBED" -B "$PROD/mbedtls-build" \
        -DCMAKE_TOOLCHAIN_FILE="$SUB/mbedtls/mbedtls-toolchain.cmake" \
        -DCMAKE_INSTALL_PREFIX="$MBED_INSTALL" -DCMAKE_BUILD_TYPE=MinSizeRel \
        -DUSE_SHARED_MBEDTLS_LIBRARY=OFF -DUSE_STATIC_MBEDTLS_LIBRARY=ON \
        -DENABLE_TESTING=OFF -DENABLE_PROGRAMS=OFF -DMBEDTLS_FATAL_WARNINGS=OFF \
        -DMBEDTLS_USER_CONFIG_FILE="$SUB/mbedtls/mbedtls-user-config.h"
      cmake --build "$PROD/mbedtls-build" --target install -j"$(nproc)"
    fi
    SRT_ENC_FLAGS="-DENABLE_ENCRYPTION=ON -DUSE_ENCLIB=mbedtls -DCMAKE_PREFIX_PATH=$MBED_INSTALL -DCMAKE_FIND_ROOT_PATH=$MBED_INSTALL"
    MBED_LIBS="-L$MBED_INSTALL/lib -lmbedtls -lmbedx509 -lmbedcrypto"
  fi
  for p in "$SUB"/patches/*.patch; do
    [ -e "$p" ] || continue
    git -C "$SRT" apply --reverse --check "$p" 2>/dev/null && echo "patch present: $(basename "$p")" || { echo "applying $(basename "$p")"; git -C "$SRT" apply "$p"; }
  done
  if [ ! -f "$SRT_INSTALL/lib/libsrt.a" ] || [ "${REBUILD:-0}" = "1" ]; then
    rm -rf "$SRT_BUILD" "$SRT_INSTALL"
    cmake -S "$SRT" -B "$SRT_BUILD" -DCMAKE_TOOLCHAIN_FILE="$SUB/arm-none-eabi.cmake" \
      -DCMAKE_INSTALL_PREFIX="$SRT_INSTALL" -DGNU=ON -DCMAKE_BUILD_TYPE=MinSizeRel \
      -DENABLE_APPS=OFF -DENABLE_SHARED=OFF -DENABLE_STATIC=ON \
      -DENABLE_UNITTESTS=OFF -DENABLE_TESTING=OFF -DENABLE_BONDING=OFF \
      -DENABLE_HEAVY_LOGGING=OFF -DENABLE_LOGGING=OFF $SRT_ENC_FLAGS \
      -DENABLE_STDCXX_SYNC=OFF -DENABLE_MONOTONIC_CLOCK=OFF -DENABLE_SOCK_CLOEXEC=OFF
    cmake --build "$SRT_BUILD" --target install -j"$(nproc)"
  fi
  SRT_LIB=$(echo "$SRT_INSTALL"/lib*/libsrt.a)
  INC="$INC -I$SRT_INSTALL/include"
fi

rm -f "$PROD"/*.o
# Substrate C compiles plain (no shim env), exactly as the staged builds did.
for f in $C_SRC; do
  $CC $ARCH $OPT $INC -std=gnu11 -c "$f" -o "$PROD/$(basename "${f%.c}").o"
done

# App + the C++ glue. cxa_override is always linked (per-task eh globals); the
# libsrt shim env (posix-shims force-include + __GNU__ + SRT_NO_PTHREAD_CANCEL +
# pthread_key_shim) is added only for LIBSRT targets (they include <srt/srt.h>).
SHIM_DEFS="${DEFS:-}"
CXX_SRC="$APP $SUB/cxa_override.cpp"
if [ "${LIBSRT:-0}" = "1" ]; then
  INC="-I$SUB/posix-shims $INC"
  SHIM_DEFS="$SHIM_DEFS -include $SUB/posix-shims/s2_prefix.h -D__GNU__=1 -DSRT_NO_PTHREAD_CANCEL"
  [ "${ENCRYPT:-0}" = "1" ] && SHIM_DEFS="$SHIM_DEFS -DSRT_PASSPHRASE=\"freertos-srt-secret-1\""
  $CC $ARCH $OPT $INC $SHIM_DEFS -std=gnu11 -c "$SUB/pthread_key_shim.c" -o "$PROD/pthread_key_shim.o"
fi
for f in $CXX_SRC; do
  $CXX $ARCH $OPT $INC $SHIM_DEFS -std=gnu++11 -fexceptions -c "$f" -o "$PROD/$(basename "${f%.cpp}").o"
done

$CXX $ARCH $OPT --specs=rdimon.specs -T "$SUB/mps2_an386.ld" -Wl,--gc-sections \
  -Wl,--wrap=clock_gettime -Wl,--wrap=setsockopt -Wl,--wrap=gettimeofday \
  -Wl,--start-group "$PROD"/*.o $SRT_LIB $MBED_LIBS -lstdc++ -Wl,--end-group \
  -o "$PROD/firmware.elf"
arm-none-eabi-size "$PROD/firmware.elf"
