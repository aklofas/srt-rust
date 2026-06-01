#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
ROOT=$(cd ../.. && pwd)
K=$ROOT/vendor/freertos-kernel
P=$ROOT/vendor/freertos-plus-posix
L=$ROOT/vendor/lwip

ARCH="-mcpu=cortex-m4 -mthumb -mfloat-abi=hard -mfpu=fpv4-sp-d16"
OPT="-Os -ffunction-sections -fdata-sections -g"

# -I. first so our FreeRTOS_POSIX_portable.h / FreeRTOS_POSIX_config.h win.
INC="-I. -I$K/include -I$K/portable/GCC/ARM_CM4F \
     -I$P/include -I$P/include/private \
     -I$P/FreeRTOS-Plus-POSIX/include -I$P/FreeRTOS-Plus-POSIX/include/portable \
     -I$L/src/include \
     -I$ROOT/crates/baremetal-qemu-c/firmware"   # golden.h (reused 564B golden)

CC=arm-none-eabi-gcc
CXX=arm-none-eabi-g++

rm -f *.o

PS=$P/FreeRTOS-Plus-POSIX/source
C_SRC="startup.c clock_shim.c $K/tasks.c $K/list.c $K/queue.c $K/timers.c $K/event_groups.c \
       $K/portable/GCC/ARM_CM4F/port.c $K/portable/MemMang/heap_4.c \
       $PS/FreeRTOS_POSIX_pthread.c $PS/FreeRTOS_POSIX_pthread_mutex.c \
       $PS/FreeRTOS_POSIX_pthread_cond.c $PS/FreeRTOS_POSIX_pthread_barrier.c \
       $PS/FreeRTOS_POSIX_clock.c $PS/FreeRTOS_POSIX_utils.c \
       $PS/FreeRTOS_POSIX_sched.c $PS/FreeRTOS_POSIX_unistd.c \
       $PS/FreeRTOS_POSIX_timer.c $PS/FreeRTOS_POSIX_semaphore.c"

# lwIP core (UDP/IPv4) + sequential/socket API + our hand-written sys_arch port.
# Glob the core/ipv4/api dirs (loopback netif lives in core/netif.c; no ethernet
# or slip files). Basenames are unique across these dirs.
LWIP_SRC="sys_arch.c $L/src/core/*.c $L/src/core/ipv4/*.c $L/src/api/*.c"

for f in $C_SRC $LWIP_SRC; do
  $CC $ARCH $OPT $INC -std=gnu11 -c "$f" -o "$(basename "${f%.c}").o"
done

CXX_SRC="main.cpp ${S1_EXTRA_SRC:-}"
for f in $CXX_SRC; do
  $CXX $ARCH $OPT $INC -std=gnu++11 -fexceptions -c "$f" -o "$(basename "${f%.cpp}").o"
done

# --wrap=clock_gettime: redirect every clock_gettime reference to the hi-res
# __wrap_clock_gettime in clock_shim.c (see that file's header for why).
$CXX $ARCH $OPT \
  --specs=rdimon.specs -T mps2_an386.ld -Wl,--gc-sections \
  -Wl,--wrap=clock_gettime \
  *.o -lstdc++ \
  -o firmware.elf

arm-none-eabi-size firmware.elf
