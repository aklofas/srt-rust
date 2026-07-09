# Bare-metal arm-none-eabi toolchain for cross-building libsrt against the
# freertos-srt FreeRTOS+lwIP substrate. CMAKE_SYSTEM_NAME=Generic (NOT Linux) so
# libsrt's POSIX var is false and it compiles the portable ::select() path, not epoll.
set(CMAKE_SYSTEM_NAME       Generic)
set(CMAKE_SYSTEM_PROCESSOR  arm)
set(CMAKE_C_COMPILER        arm-none-eabi-gcc)
set(CMAKE_CXX_COMPILER      arm-none-eabi-g++)

# CMake's compiler sanity probe must build a static lib, not a semihosted exe
# (no _exit/OS at configure time).
set(CMAKE_TRY_COMPILE_TARGET_TYPE STATIC_LIBRARY)

get_filename_component(_SUB "${CMAKE_CURRENT_LIST_DIR}" ABSOLUTE)
# This file lives at embedded/freertos-srt/substrate/ — three levels under the
# workspace root. Embedded-only vendor trees live at embedded/vendor/; the
# shared srt/mbedtls submodules stay at the workspace-level vendor/.
set(_ROOT "${_SUB}/../../..")
set(_K "${_ROOT}/embedded/vendor/freertos-kernel")
set(_P "${_ROOT}/embedded/vendor/freertos-plus-posix")
set(_L "${_ROOT}/embedded/vendor/lwip")

set(_ARCH "-mcpu=cortex-m4 -mthumb -mfloat-abi=hard -mfpu=fpv4-sp-d16")
# -D__GNU__: selects libsrt utilities.h's endian branch
# (__linux__||__CYGWIN__||__GNU__||__GLIBC__) — the only non-Linux option that
# doesn't also pull epoll — so it #includes our posix-shims/endian.h instead of
# hitting its unconditional `#error Endian: platform not supported`.
# (libsrt's single pthread_cancel call — absent from FreeRTOS-Plus-POSIX — is
# removed by the vendor/srt patch, which makes CThread assign-to-joinable
# std::terminate() like std::thread; no opt-out define needed.)
set(_DEFS "-D__GNU__=1 -include ${CMAKE_CURRENT_LIST_DIR}/posix-shims/shim_prefix.h")
# The cross-built libsrt (USE_ENCLIB=mbedtls) includes mbedTLS headers and MUST
# see the same configuration view the mbedTLS library itself was compiled with
# (mbedtls-user-config.h: entropy routed to mbedtls_hardware_poll, OS modules
# off) — mbedTLS's own build receives the file as a CMake var in
# build-common.sh; consumers must get it as a preprocessor define. Inert for
# ENCRYPT=0 builds (no mbedTLS header is included anywhere).
set(_DEFS "${_DEFS} -DMBEDTLS_USER_CONFIG_FILE=\\\"${CMAKE_CURRENT_LIST_DIR}/mbedtls/mbedtls-user-config.h\\\"")
# Include order matters: posix-shims FIRST (so <pthread.h>/<netinet/in.h> route
# to FreeRTOS-Plus-POSIX/lwIP), then the substrate config dirs (substrate/lwip
# has lwipopts.h + arch/cc.h; substrate/freertos has FreeRTOSConfig.h), then
# FreeRTOS + lwIP. lwIP's compat/posix supplies <sys/socket.h>/<arpa/inet.h>.
set(_INC "-I${_SUB}/posix-shims -I${_SUB} -I${_SUB}/freertos -I${_SUB}/lwip \
 -I${_K}/include -I${_K}/portable/GCC/ARM_CM4F \
 -I${_P}/include -I${_P}/include/private \
 -I${_P}/FreeRTOS-Plus-POSIX/include -I${_P}/FreeRTOS-Plus-POSIX/include/portable \
 -I${_L}/src/include -I${_L}/src/include/compat/posix")

# Vendored-libsrt warning suppressions, scoped to this cross-build (the pinned
# v1.5.5 source is not ours to fix):
# -Wno-type-limits: core.cpp:541 `int(optName) < 0` is always-false on this
#   target's enum representation — upstream-portable code, noise here.
# -Wno-cpp: epoll.cpp's `#warning` that epoll is unsupported — expected and
#   intentional; CMAKE_SYSTEM_NAME=Generic selects the portable ::select()
#   path by design (see header comment above).
set(_WNO "-Wno-type-limits -Wno-cpp")
# Section-per-function/data so the firmware link's --gc-sections can drop
# unused library code — without these the whole archive member survives.
set(_GC "-ffunction-sections -fdata-sections")
set(CMAKE_C_FLAGS   "${_ARCH} ${_DEFS} ${_INC} ${_GC}" CACHE STRING "")
set(CMAKE_CXX_FLAGS "${_ARCH} ${_DEFS} ${_INC} ${_WNO} ${_GC}" CACHE STRING "")

set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_PACKAGE ONLY)
