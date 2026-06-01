#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
ROOT=$(cd ../.. && pwd)
K=$ROOT/vendor/freertos-kernel

ARCH="-mcpu=cortex-m4 -mthumb -mfloat-abi=hard -mfpu=fpv4-sp-d16"
OPT="-Os -ffunction-sections -fdata-sections -g"
INC="-I. -I$K/include -I$K/portable/GCC/ARM_CM4F"

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

C_SRC="startup.c $K/tasks.c $K/list.c $K/queue.c $K/timers.c \
       $K/portable/GCC/ARM_CM4F/port.c $K/portable/MemMang/heap_4.c"
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
