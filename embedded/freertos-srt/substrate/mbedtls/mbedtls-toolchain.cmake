# Minimal bare-metal arm-none-eabi toolchain for cross-building vendored mbedTLS
# (Phase B). mbedTLS is plain C and, unlike arm-none-eabi.cmake, needs NONE of
# the libsrt/lwIP/FreeRTOS includes or the s2_prefix force-include (those
# suppress newlib typedefs for the libsrt-vs-FreeRTOS-POSIX collision and would
# only get in mbedTLS's way) — only the Cortex-M4F arch flags. The bare-metal
# config deltas (no NET/FS/TIMING; hardware-entropy alt) are passed via
# -DMBEDTLS_USER_CONFIG_FILE, which mbedTLS's CMakeLists quotes correctly.
set(CMAKE_SYSTEM_NAME       Generic)
set(CMAKE_SYSTEM_PROCESSOR  arm)
set(CMAKE_C_COMPILER        arm-none-eabi-gcc)
# CMake's compiler sanity probe must build a static lib, not a semihosted exe.
set(CMAKE_TRY_COMPILE_TARGET_TYPE STATIC_LIBRARY)
set(CMAKE_C_FLAGS "-mcpu=cortex-m4 -mthumb -mfloat-abi=hard -mfpu=fpv4-sp-d16 -Os -ffunction-sections -fdata-sections" CACHE STRING "")
set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)
