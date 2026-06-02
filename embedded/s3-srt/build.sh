#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
ROOT=$(cd ../.. && pwd)
K=$ROOT/vendor/freertos-kernel
P=$ROOT/vendor/freertos-plus-posix
L=$ROOT/vendor/lwip

ARCH="-mcpu=cortex-m4 -mthumb -mfloat-abi=hard -mfpu=fpv4-sp-d16"
OPT="-Os -ffunction-sections -fdata-sections -g"

# posix-shims FIRST (so <pthread.h>/<netinet/in.h>/<sys/ioctl.h> route to
# FreeRTOS-Plus-POSIX / lwIP), then -I. (lwipopts.h, arch/, our config headers
# win), then FreeRTOS + lwIP. lwIP's compat/posix supplies <sys/socket.h> etc.
INC="-Iposix-shims -I. -I$K/include -I$K/portable/GCC/ARM_CM4F \
     -I$P/include -I$P/include/private \
     -I$P/FreeRTOS-Plus-POSIX/include -I$P/FreeRTOS-Plus-POSIX/include/portable \
     -I$L/src/include -I$L/src/include/compat/posix \
     -I$ROOT/crates/baremetal-qemu-c/firmware"   # golden.h (reused 564B golden)

CC=arm-none-eabi-gcc
CXX=arm-none-eabi-g++

SRT=$ROOT/vendor/srt
SRT_BUILD=srt-build
SRT_INSTALL=$(pwd)/srt-install

# Apply our bare-metal portability patches to the pinned vendor/srt submodule.
# The submodule stays pristine in git (pointer unchanged); the patches live in
# our tree so CI's clean recursive checkout gets them too. Idempotent: skip any
# patch that already reverse-applies cleanly (i.e. is already in place).
for p in "$(pwd)"/patches/*.patch; do
  [ -e "$p" ] || continue
  if git -C "$SRT" apply --reverse --check "$p" 2>/dev/null; then
    echo "patch already applied: $(basename "$p")"
  else
    echo "applying patch: $(basename "$p")"
    git -C "$SRT" apply "$p"
  fi
done

# Cross-build libsrt.a for the target (CMAKE_SYSTEM_NAME=Generic → ::select()
# path; pthread backend; logging/encryption OFF; mirrors srt-sys/build.rs opts).
if [ ! -f "$SRT_INSTALL/lib/libsrt.a" ] || [ "${S3_REBUILD_SRT:-0}" = "1" ]; then
  rm -rf "$SRT_BUILD" "$SRT_INSTALL"
  # -DGNU=ON: libsrt's system-detection block (CMakeLists.txt ~L829) FATALs on
  # any CMAKE_SYSTEM_NAME it doesn't recognize, and the `elseif(GNU)` branch
  # checks a `GNU` var that libsrt actually renamed to `GNU_OS` (L47 comment) —
  # so even Generic/GNU systems hit the `else()` FATAL. Forcing GNU=ON selects
  # that branch, which only `add_definitions(-DGNU=1)` (unused in any C/C++
  # source — verified) and, unlike the LINUX/BSD branches, does NOT pull epoll
  # or kqueue. With LINUX undefined, epoll.cpp + platform_sys.h compile the
  # portable ::select() path against lwIP (the whole point of S2).
  cmake -S "$SRT" -B "$SRT_BUILD" \
    -DCMAKE_TOOLCHAIN_FILE="$(pwd)/arm-none-eabi.cmake" \
    -DCMAKE_INSTALL_PREFIX="$SRT_INSTALL" \
    -DGNU=ON \
    -DCMAKE_BUILD_TYPE=MinSizeRel \
    -DENABLE_APPS=OFF -DENABLE_SHARED=OFF -DENABLE_STATIC=ON \
    -DENABLE_UNITTESTS=OFF -DENABLE_TESTING=OFF -DENABLE_BONDING=OFF \
    -DENABLE_HEAVY_LOGGING=OFF -DENABLE_LOGGING=OFF -DENABLE_ENCRYPTION=OFF \
    -DENABLE_STDCXX_SYNC=OFF -DENABLE_MONOTONIC_CLOCK=OFF \
    -DENABLE_SOCK_CLOEXEC=OFF
  cmake --build "$SRT_BUILD" --target install -j"$(nproc)"
fi
SRT_LIB=$(echo "$SRT_INSTALL"/lib*/libsrt.a)   # lib or lib64
arm-none-eabi-size "$SRT_LIB" | tail -1

# main.cpp includes <srt/srt.h> from the install tree.
INC="$INC -I$SRT_INSTALL/include"

rm -f *.o

PS=$P/FreeRTOS-Plus-POSIX/source
# Substrate (FreeRTOS + lwIP + startup/clock) — compiled WITHOUT the posix-shim
# env, exactly as in S1 (these don't include srt or our pthread.h shim).
# lossy_netif.c is pure lwIP (no srt/shim headers) so it belongs here.
C_SRC="startup.c clock_shim.c syscalls_stub.c atomic64_stub.c lossy_netif.c net_shim.c $K/tasks.c $K/list.c $K/queue.c $K/timers.c $K/event_groups.c \
       $K/portable/GCC/ARM_CM4F/port.c $K/portable/MemMang/heap_4.c \
       $PS/FreeRTOS_POSIX_pthread.c $PS/FreeRTOS_POSIX_pthread_mutex.c \
       $PS/FreeRTOS_POSIX_pthread_cond.c $PS/FreeRTOS_POSIX_pthread_barrier.c \
       $PS/FreeRTOS_POSIX_clock.c $PS/FreeRTOS_POSIX_utils.c \
       $PS/FreeRTOS_POSIX_sched.c $PS/FreeRTOS_POSIX_unistd.c \
       $PS/FreeRTOS_POSIX_timer.c $PS/FreeRTOS_POSIX_semaphore.c"

# lwIP core (UDP/IPv4 + IPv6 for libsrt's sockaddr_in6/ip6 types) + sequential/
# socket API + our hand-written sys_arch port. Basenames are unique across dirs.
LWIP_SRC="sys_arch.c $L/src/core/*.c $L/src/core/ipv4/*.c $L/src/core/ipv6/*.c $L/src/api/*.c"

for f in $C_SRC $LWIP_SRC; do
  $CC $ARCH $OPT $INC -std=gnu11 -c "$f" -o "$(basename "${f%.c}").o"
done

# App sources (our pthread_key TSD shim + main.cpp) include the posix-shim
# headers / srt.h, so they need the SAME compat env libsrt was built with:
# the s2_prefix.h force-include (newlib type suppressions), -D__GNU__ (endian
# branch), and -DSRT_NO_PTHREAD_CANCEL (matches the vendor/srt patch).
SHIM_DEFS="-include posix-shims/s2_prefix.h -D__GNU__=1 -DSRT_NO_PTHREAD_CANCEL -DS3_LOSS_ENABLED=1"
$CC $ARCH $OPT $INC $SHIM_DEFS -std=gnu11 -c pthread_key_shim.c -o pthread_key_shim.o

# cxa_override.cpp: strong __cxa_get_globals/_fast defs (per-task eh state in
# FreeRTOS TLS slot 1) — required because libsrt's API throws from our pthreads.
CXX_SRC="main.cpp cxa_override.cpp ${S3_EXTRA_SRC:-}"
for f in $CXX_SRC; do
  $CXX $ARCH $OPT $INC $SHIM_DEFS -std=gnu++11 -fexceptions -c "$f" -o "$(basename "${f%.cpp}").o"
done

# --wrap=clock_gettime: redirect every clock_gettime reference to the hi-res
# __wrap_clock_gettime in clock_shim.c (see that file's header for why).
# Link libsrt.a (after our .o, before libstdc++). --start-group wraps libsrt +
# libstdc++ + libc so their cross-references resolve regardless of order.
$CXX $ARCH $OPT \
  --specs=rdimon.specs -T mps2_an386.ld -Wl,--gc-sections \
  -Wl,--wrap=clock_gettime -Wl,--wrap=setsockopt -Wl,--wrap=gettimeofday \
  -Wl,--start-group *.o "$SRT_LIB" -lstdc++ -Wl,--end-group \
  -o firmware.elf

arm-none-eabi-size firmware.elf
