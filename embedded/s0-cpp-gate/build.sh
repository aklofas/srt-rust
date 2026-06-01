#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
ROOT=$(cd ../.. && pwd)
K=$ROOT/vendor/freertos-kernel
P=$ROOT/vendor/freertos-plus-posix

ARCH="-mcpu=cortex-m4 -mthumb -mfloat-abi=hard -mfpu=fpv4-sp-d16"
OPT="-Os -ffunction-sections -fdata-sections -g"
# -I. first so our FreeRTOS_POSIX_portable.h / FreeRTOS_POSIX_config.h win.
# FreeRTOS-Plus-POSIX include layout: the public POSIX headers live under
# include/ (so <FreeRTOS_POSIX/pthread.h> resolves), the wrapper's own headers
# (FreeRTOS_POSIX.h etc.) live under FreeRTOS-Plus-POSIX/include/, and its
# private list helper under include/private.
INC="-I. -I$K/include -I$K/portable/GCC/ARM_CM4F \
     -I$P/include -I$P/include/private \
     -I$P/FreeRTOS-Plus-POSIX/include -I$P/FreeRTOS-Plus-POSIX/include/portable"

# FreeRTOS kernel + startup are C — compile with the C frontend so g++'s
# stricter void*->T* rules don't reject the kernel macros. C++ sources
# (main.cpp + cxa_override.cpp + any S0_EXTRA_SRC) compile with the C++
# frontend. cxa_override.cpp is the crux of the gate — its strong
# __cxa_get_globals{,_fast} defs must be in the link (and ahead of -lstdc++,
# which the *.o glob below guarantees) to win over libsupc++'s single-threaded
# versions. To run the Task 7 RED-proof, drop it from CXX_SRC below.
CC=arm-none-eabi-gcc
CXX=arm-none-eabi-g++

rm -f *.o

# Kernel: + event_groups.c (FreeRTOS+POSIX cond/barrier reference it).
# FreeRTOS+POSIX: the pthread + sync wrappers libsrt's sync_posix.cpp binds to,
# plus their support TUs (clock provides clock_gettime — no stub needed; utils
# does the timespec<->tick math; sched/unistd round out the surface). mqueue is
# omitted (unused by the gate). These are C — compile with the C frontend.
PS=$P/FreeRTOS-Plus-POSIX/source
C_SRC="startup.c $K/tasks.c $K/list.c $K/queue.c $K/timers.c $K/event_groups.c \
       $K/portable/GCC/ARM_CM4F/port.c $K/portable/MemMang/heap_4.c \
       $PS/FreeRTOS_POSIX_pthread.c $PS/FreeRTOS_POSIX_pthread_mutex.c \
       $PS/FreeRTOS_POSIX_pthread_cond.c $PS/FreeRTOS_POSIX_pthread_barrier.c \
       $PS/FreeRTOS_POSIX_clock.c $PS/FreeRTOS_POSIX_utils.c \
       $PS/FreeRTOS_POSIX_sched.c $PS/FreeRTOS_POSIX_unistd.c \
       $PS/FreeRTOS_POSIX_timer.c $PS/FreeRTOS_POSIX_semaphore.c"
for f in $C_SRC; do
  $CC $ARCH $OPT $INC -std=gnu11 -c "$f" -o "$(basename "${f%.c}").o"
done

CXX_SRC="main.cpp cxa_override.cpp ${S0_EXTRA_SRC:-}"
for f in $CXX_SRC; do
  $CXX $ARCH $OPT $INC -std=gnu++11 -fexceptions -c "$f" -o "$(basename "${f%.cpp}").o"
done

# Link with g++ so libstdc++ + the C++ unwinder come in. rdimon provides
# semihosting syscalls. NOTE: we deliberately do NOT use --specs=nano.specs:
# nano.specs substitutes libstdc++_nano.a, whose stripped eh_globals/eh_alloc
# objects make a throw/catch land in std::terminate on this toolchain. The
# full libstdc++.a (plus full newlib) gives a working two-phase unwind.
$CXX $ARCH $OPT \
  --specs=rdimon.specs -T mps2_an386.ld -Wl,--gc-sections \
  *.o -lstdc++ \
  -o firmware.elf

arm-none-eabi-size firmware.elf
