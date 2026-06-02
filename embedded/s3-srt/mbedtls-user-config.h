/* Appended on top of mbedTLS's default config (via MBEDTLS_USER_CONFIG_FILE)
 * for the bare-metal arm-none-eabi build. Drop the OS-dependent modules (no
 * filesystem, no BSD-net layer, no OS timing/clock) and route entropy to a
 * hardware-poll hook (mbedtls_hardware_poll, provided in syscalls_stub.c).
 * libsrt's haicrypt needs AES + SHA + PBKDF2 + CTR_DRBG, all of which stay on. */
#undef MBEDTLS_NET_C
#undef MBEDTLS_FS_IO
#undef MBEDTLS_TIMING_C
#undef MBEDTLS_HAVE_TIME
#undef MBEDTLS_HAVE_TIME_DATE
#undef MBEDTLS_PSA_ITS_FILE_C
#undef MBEDTLS_PSA_CRYPTO_STORAGE_C
#define MBEDTLS_NO_PLATFORM_ENTROPY
#define MBEDTLS_ENTROPY_HARDWARE_ALT
