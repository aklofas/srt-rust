# Bare-metal arm-none-eabi toolchain for cross-building libsrt against the S2
# FreeRTOS+lwIP substrate. CMAKE_SYSTEM_NAME=Generic (NOT Linux) so libsrt's
# POSIX var is false and it compiles the portable ::select() path, not epoll.
set(CMAKE_SYSTEM_NAME       Generic)
set(CMAKE_SYSTEM_PROCESSOR  arm)
set(CMAKE_C_COMPILER        arm-none-eabi-gcc)
set(CMAKE_CXX_COMPILER      arm-none-eabi-g++)

# CMake's compiler sanity probe must build a static lib, not a semihosted exe
# (no _exit/OS at configure time).
set(CMAKE_TRY_COMPILE_TARGET_TYPE STATIC_LIBRARY)

get_filename_component(_S2 "${CMAKE_CURRENT_LIST_DIR}" ABSOLUTE)
# This file lives at embedded/freertos-srt/substrate/ — three levels under the
# workspace root (was embedded/s4-srt/, two levels, hence the old ../..).
set(_ROOT "${_S2}/../../..")
set(_K "${_ROOT}/vendor/freertos-kernel")
set(_P "${_ROOT}/vendor/freertos-plus-posix")
set(_L "${_ROOT}/vendor/lwip")

set(_ARCH "-mcpu=cortex-m4 -mthumb -mfloat-abi=hard -mfpu=fpv4-sp-d16")
# -D__GNU__: selects libsrt utilities.h's endian branch
# (__linux__||__CYGWIN__||__GNU__||__GLIBC__) — the only non-Linux option that
# doesn't also pull epoll — so it #includes our posix-shims/endian.h instead of
# hitting its unconditional `#error Endian: platform not supported`.
# -DSRT_NO_PTHREAD_CANCEL: FreeRTOS-Plus-POSIX has no pthread_cancel; a one-line
# vendor/srt patch guards its single use (an IPE error path) on this define.
set(_DEFS "-D__GNU__=1 -DSRT_NO_PTHREAD_CANCEL -include ${CMAKE_CURRENT_LIST_DIR}/posix-shims/s2_prefix.h")
# Include order matters: posix-shims FIRST (so <pthread.h>/<netinet/in.h> route
# to FreeRTOS-Plus-POSIX/lwIP), then the substrate config dirs (substrate/lwip
# has lwipopts.h + arch/cc.h; substrate/freertos has FreeRTOSConfig.h), then
# FreeRTOS + lwIP. lwIP's compat/posix supplies <sys/socket.h>/<arpa/inet.h>.
set(_INC "-I${_S2}/posix-shims -I${_S2} -I${_S2}/freertos -I${_S2}/lwip \
 -I${_K}/include -I${_K}/portable/GCC/ARM_CM4F \
 -I${_P}/include -I${_P}/include/private \
 -I${_P}/FreeRTOS-Plus-POSIX/include -I${_P}/FreeRTOS-Plus-POSIX/include/portable \
 -I${_L}/src/include -I${_L}/src/include/compat/posix")

set(CMAKE_C_FLAGS   "${_ARCH} ${_DEFS} ${_INC}" CACHE STRING "")
set(CMAKE_CXX_FLAGS "${_ARCH} ${_DEFS} ${_INC}" CACHE STRING "")

set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_PACKAGE ONLY)
