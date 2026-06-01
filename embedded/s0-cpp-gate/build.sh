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
# (main.cpp + any S0_EXTRA_SRC) compile with the C++ frontend.
CC=arm-none-eabi-gcc
CXX=arm-none-eabi-g++

rm -f *.o

C_SRC="startup.c $K/tasks.c $K/list.c $K/queue.c $K/timers.c \
       $K/portable/GCC/ARM_CM4F/port.c $K/portable/MemMang/heap_4.c"
for f in $C_SRC; do
  $CC $ARCH $OPT $INC -std=gnu11 -c "$f" -o "$(basename "${f%.c}").o"
done

CXX_SRC="main.cpp ${S0_EXTRA_SRC:-}"
for f in $CXX_SRC; do
  $CXX $ARCH $OPT $INC -std=gnu++11 -fexceptions -c "$f" -o "$(basename "${f%.cpp}").o"
done

# Link with g++ so libstdc++ + the C++ unwinder come in. rdimon provides
# semihosting syscalls; nano.specs gives newlib-nano for the C side.
$CXX $ARCH $OPT \
  --specs=nano.specs --specs=rdimon.specs -T mps2_an386.ld -Wl,--gc-sections \
  *.o -lstdc++ \
  -o firmware.elf

arm-none-eabi-size firmware.elf
