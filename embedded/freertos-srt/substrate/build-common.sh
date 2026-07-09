#!/usr/bin/env bash
# Parameterized firmware build for the freertos-srt product. Sourced by the
# top-level build.sh dispatcher, which sets the per-target knobs:
#   LWIP=0|1  LIBSRT=0|1  NETIF=none|lossy|lan9118  ENCRYPT=0|1
#   APP="<app .cpp ...>"  DEFS="<extra -D...>"
# All generated artifacts (objects, firmware.elf, generated headers, the staged
# libsrt source, and the cross-built libsrt/mbedTLS trees) land under a single
# ignored build/ directory so the product root stays easy to scan. `REBUILD=1`
# forces a clean libsrt/mbedTLS rebuild; the cross-build also self-invalidates
# when the toolchain files, patches, config headers, or vendored submodule HEADs
# change (a stamp file under build/), so a stale static lib can't survive an
# input edit. Wipe everything with: rm -rf build/  (or build.sh clean).
set -euo pipefail
PROD=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)   # embedded/freertos-srt
SUB="$PROD/substrate"
ROOT=$(cd "$PROD/../.." && pwd)                          # workspace root
cd "$PROD"

# Single ignored output root. obj/ = compiled objects; generated/ = golden.h;
# srt-src/ = staged pristine libsrt source (patched there, not in vendor/srt);
# srt-build*/srt-install*/mbedtls-* = the cross-built dependency trees.
BUILD="$PROD/build"
OBJ="$BUILD/obj"
GEN="$BUILD/generated"
mkdir -p "$BUILD" "$OBJ" "$GEN"

K=$ROOT/embedded/vendor/freertos-kernel
P=$ROOT/embedded/vendor/freertos-plus-posix
PS=$P/FreeRTOS-Plus-POSIX/source
L=$ROOT/embedded/vendor/lwip
SRT=$ROOT/vendor/srt
MBED=$ROOT/vendor/mbedtls
CC=arm-none-eabi-gcc; CXX=arm-none-eabi-g++

ARCH="-mcpu=cortex-m4 -mthumb -mfloat-abi=hard -mfpu=fpv4-sp-d16"
OPT="-Os -ffunction-sections -fdata-sections -g"

# Generate golden.h (564B video-roundtrip) into build/generated/ if absent;
# targets that #include it get -I$GEN. Self-contained on a clean checkout.
GOLDEN_TS=$ROOT/crates/tst-integration/tests/fixtures/scenarios/video-roundtrip/output.ts
if [ ! -f "$GEN/golden.h" ]; then
  python3 - "$GOLDEN_TS" > "$GEN/golden.h" <<'PY'
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
INC="-I$SUB -I$GEN -I$SUB/freertos -I$SUB/lwip -I$SUB/drivers \
     -I$K/include -I$K/portable/GCC/ARM_CM4F \
     -I$P/include -I$P/include/private \
     -I$P/FreeRTOS-Plus-POSIX/include -I$P/FreeRTOS-Plus-POSIX/include/portable"

# Substrate C always present: startup, clock, stubs, FreeRTOS kernel + POSIX.
# (net_shim.c is the libsrt-on-lwIP __wrap_setsockopt shim — it needs lwIP
# headers and is only meaningful on the data-plane targets, so it rides with the
# netif drivers below, exactly as in loopback-arq/example. The non-netif targets never call
# setsockopt, so -Wl,--wrap=setsockopt stays inert for them.)
C_SRC="$SUB/startup.c $SUB/diag.c $SUB/newlib_lock.c $SUB/clock_shim.c $SUB/syscalls_stub.c $SUB/atomic64_stub.c \
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
# don't clobber. Only for LIBSRT targets. Everything lands under build/.
SRT_LIB=""; MBED_LIBS=""; MBED_INSTALL="$BUILD/mbedtls-install"
if [ "${LIBSRT:-0}" = "1" ]; then
  SRT_INSTALL="$BUILD/srt-install${ENCRYPT:-0}"; SRT_BUILD="$BUILD/srt-build${ENCRYPT:-0}"
  SRT_SRC="$BUILD/srt-src"

  # Cache invalidation: a stamp hashing the toolchain files, the patch set, the
  # mbedTLS user-config, the vendored libsrt/mbedTLS submodule HEADs, AND this
  # script itself (the libsrt/mbedTLS CMake flags — ENABLE_*, build type, link
  # deps — are hardcoded below, so an edit to them must bust the cache too). When
  # it changes, blow away the staged source + both ENCRYPT trees so an input edit
  # can never link a stale static lib. (Per-tree existence checks below still
  # cover the first build of each ENCRYPT variant under an unchanged stamp.)
  STAMPFILE="$BUILD/.cross-build-stamp"
  NEWSTAMP=$( { sha256sum "${BASH_SOURCE[0]}" "$SUB/arm-none-eabi.cmake" "$SUB/mbedtls/mbedtls-toolchain.cmake" \
                  "$SUB/mbedtls/mbedtls-user-config.h" "$SUB"/patches/*.patch 2>/dev/null
               git -C "$SRT" rev-parse HEAD; git -C "$MBED" rev-parse HEAD; } | sha256sum | awk '{print $1}')
  EFF_REBUILD="${REBUILD:-0}"
  if [ "$NEWSTAMP" != "$(cat "$STAMPFILE" 2>/dev/null || true)" ]; then
    echo "freertos-srt: cross-build inputs changed -> rebuilding libsrt/mbedTLS"
    rm -rf "$BUILD"/srt-build* "$BUILD"/srt-install* "$BUILD/mbedtls-build" "$MBED_INSTALL" "$SRT_SRC"
    EFF_REBUILD=1
  fi

  SRT_ENC_FLAGS="-DENABLE_ENCRYPTION=OFF"
  if [ "${ENCRYPT:-0}" = "1" ]; then
    if [ ! -f "$MBED_INSTALL/lib/libmbedcrypto.a" ] || [ "$EFF_REBUILD" = "1" ]; then
      rm -rf "$BUILD/mbedtls-build" "$MBED_INSTALL"
      # CMAKE_WARN_DEPRECATED=OFF: the pinned submodule declares
      # cmake_minimum_required 3.5.1 — newer CMakes print a deprecation
      # banner about it that we can't act on without a submodule bump.
      cmake -S "$MBED" -B "$BUILD/mbedtls-build" \
        -DCMAKE_TOOLCHAIN_FILE="$SUB/mbedtls/mbedtls-toolchain.cmake" \
        -DCMAKE_INSTALL_PREFIX="$MBED_INSTALL" -DCMAKE_BUILD_TYPE=MinSizeRel \
        -DUSE_SHARED_MBEDTLS_LIBRARY=OFF -DUSE_STATIC_MBEDTLS_LIBRARY=ON \
        -DENABLE_TESTING=OFF -DENABLE_PROGRAMS=OFF -DMBEDTLS_FATAL_WARNINGS=OFF \
        -DCMAKE_WARN_DEPRECATED=OFF \
        -DMBEDTLS_USER_CONFIG_FILE="$SUB/mbedtls/mbedtls-user-config.h"
      cmake --build "$BUILD/mbedtls-build" --target install -j"$(nproc)"
    fi
    SRT_ENC_FLAGS="-DENABLE_ENCRYPTION=ON -DUSE_ENCLIB=mbedtls -DCMAKE_PREFIX_PATH=$MBED_INSTALL -DCMAKE_FIND_ROOT_PATH=$MBED_INSTALL"
    MBED_LIBS="-L$MBED_INSTALL/lib -lmbedtls -lmbedx509 -lmbedcrypto"
  fi

  # Stage a pristine copy of the pinned libsrt source and patch it THERE — never
  # mutate vendor/srt (shared submodule; an interrupted in-place patch left it
  # dirty for every other consumer). `git archive HEAD` gives exactly the tracked
  # source at the pinned commit. Apply with plain `patch`, NOT `git apply`: the
  # staged tree sits under build/ inside this repo's worktree, so `git apply`
  # would discover the parent .git and apply in repo-context (a silent no-op on
  # the staged files). `patch -p1` is repo-agnostic and writes the files directly.
  #
  # Crash-safe: stage + patch into a temp dir, then atomically rename into place.
  # srt-src therefore only ever exists as a complete, fully-patched tree — an
  # interrupted run leaves only the temp dir (wiped on the next run), so a partial
  # tree can never be silently reused.
  if [ ! -d "$SRT_SRC" ] || [ "$EFF_REBUILD" = "1" ]; then
    STAGE="$BUILD/.srt-src.staging"
    rm -rf "$STAGE" "$SRT_SRC"; mkdir -p "$STAGE"
    git -C "$SRT" archive HEAD | tar -x -C "$STAGE"
    for p in "$SUB"/patches/*.patch; do
      [ -e "$p" ] || continue
      echo "applying $(basename "$p") to staged srt-src"
      ( cd "$STAGE" && patch -p1 -s < "$p" )
    done
    mv "$STAGE" "$SRT_SRC"
  fi

  if [ ! -f "$SRT_INSTALL/lib/libsrt.a" ] || [ "$EFF_REBUILD" = "1" ]; then
    rm -rf "$SRT_BUILD" "$SRT_INSTALL"
    # CMAKE_WARN_DEPRECATED=OFF: pinned libsrt declares cmake_minimum_required
    # 3.5 — same unactionable deprecation banner as the mbedTLS build above.
    cmake -S "$SRT_SRC" -B "$SRT_BUILD" -DCMAKE_TOOLCHAIN_FILE="$SUB/arm-none-eabi.cmake" \
      -DCMAKE_INSTALL_PREFIX="$SRT_INSTALL" -DGNU=ON -DCMAKE_BUILD_TYPE=MinSizeRel \
      -DENABLE_APPS=OFF -DENABLE_SHARED=OFF -DENABLE_STATIC=ON \
      -DENABLE_UNITTESTS=OFF -DENABLE_TESTING=OFF -DENABLE_BONDING=OFF \
      -DENABLE_HEAVY_LOGGING=OFF -DENABLE_LOGGING=OFF $SRT_ENC_FLAGS \
      -DCMAKE_WARN_DEPRECATED=OFF \
      -DENABLE_STDCXX_SYNC=OFF -DENABLE_MONOTONIC_CLOCK=OFF -DENABLE_SOCK_CLOEXEC=OFF
    cmake --build "$SRT_BUILD" --target install -j"$(nproc)"
  fi
  printf '%s\n' "$NEWSTAMP" > "$STAMPFILE"
  SRT_LIB=$(echo "$SRT_INSTALL"/lib*/libsrt.a)
  INC="$INC -I$SRT_INSTALL/include"
fi

rm -rf "$OBJ"; mkdir -p "$OBJ"
# Substrate C compiles plain (no shim env), exactly as the staged builds did.
# $DEFS (the per-target lwipopts/behavior knobs the staged builds baked into
# their config headers — e.g. -DLWIP_NETIF_LOOPBACK=1) MUST reach these C files
# too: lwipopts.h is read by the lwIP core here, not just by the C++ app, so a
# -D that only hit the app would leave lwIP compiled with the superset default.
for f in $C_SRC; do
  $CC $ARCH $OPT $INC ${DEFS:-} -std=gnu11 -c "$f" -o "$OBJ/$(basename "${f%.c}").o"
done

# App + the C++ glue. cxa_override is always linked (per-task eh globals); the
# libsrt shim env (posix-shims force-include + __GNU__ + pthread_key_shim) is
# added only for LIBSRT targets (they include <srt/srt.h>).
SHIM_DEFS="${DEFS:-}"
CXX_SRC="$APP $SUB/cxa_override.cpp"
if [ "${LIBSRT:-0}" = "1" ]; then
  INC="-I$SUB/posix-shims $INC"
  SHIM_DEFS="$SHIM_DEFS -include $SUB/posix-shims/shim_prefix.h -D__GNU__=1"
  [ "${ENCRYPT:-0}" = "1" ] && SHIM_DEFS="$SHIM_DEFS -DSRT_PASSPHRASE=\"freertos-srt-secret-1\""
  $CC $ARCH $OPT $INC $SHIM_DEFS -std=gnu11 -c "$SUB/pthread_key_shim.c" -o "$OBJ/pthread_key_shim.o"
fi
for f in $CXX_SRC; do
  $CXX $ARCH $OPT $INC $SHIM_DEFS -std=gnu++11 -fexceptions -c "$f" -o "$OBJ/$(basename "${f%.cpp}").o"
done

# --wrap=clock_gettime + --wrap=gettimeofday: their wrappers live in clock_shim.c
# + syscalls_stub.c (always compiled), so always safe. --wrap=setsockopt's
# wrapper is __wrap_setsockopt in net_shim.c, compiled only for the data-plane
# targets (NETIF != none); the non-netif targets must NOT wrap setsockopt —
# libsrt-smoke pulls setsockopt from libsrt.a and would hit an undefined
# __wrap_setsockopt otherwise (matching libsrt-smoke, which wrapped only clock_gettime).
WRAPS="-Wl,--wrap=clock_gettime -Wl,--wrap=gettimeofday"
[ "${NETIF:-none}" != "none" ] && WRAPS="$WRAPS -Wl,--wrap=setsockopt"
$CXX $ARCH $OPT --specs=rdimon.specs -T "$SUB/mps2_an386.ld" -Wl,--gc-sections \
  $WRAPS \
  -Wl,--start-group "$OBJ"/*.o $SRT_LIB $MBED_LIBS -lstdc++ -Wl,--end-group \
  -o "$BUILD/firmware.elf"
arm-none-eabi-size "$BUILD/firmware.elf"
