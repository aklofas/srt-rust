# Install script for directory: /home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include

# Set the install prefix
if(NOT DEFINED CMAKE_INSTALL_PREFIX)
  set(CMAKE_INSTALL_PREFIX "/home/aklofas/Projects/ts-transformer/ts-transformer/embedded/s4-srt/mbedtls-install")
endif()
string(REGEX REPLACE "/$" "" CMAKE_INSTALL_PREFIX "${CMAKE_INSTALL_PREFIX}")

# Set the install configuration name.
if(NOT DEFINED CMAKE_INSTALL_CONFIG_NAME)
  if(BUILD_TYPE)
    string(REGEX REPLACE "^[^A-Za-z0-9_]+" ""
           CMAKE_INSTALL_CONFIG_NAME "${BUILD_TYPE}")
  else()
    set(CMAKE_INSTALL_CONFIG_NAME "MinSizeRel")
  endif()
  message(STATUS "Install configuration: \"${CMAKE_INSTALL_CONFIG_NAME}\"")
endif()

# Set the component getting installed.
if(NOT CMAKE_INSTALL_COMPONENT)
  if(COMPONENT)
    message(STATUS "Install component: \"${COMPONENT}\"")
    set(CMAKE_INSTALL_COMPONENT "${COMPONENT}")
  else()
    set(CMAKE_INSTALL_COMPONENT)
  endif()
endif()

# Is this installation the result of a crosscompile?
if(NOT DEFINED CMAKE_CROSSCOMPILING)
  set(CMAKE_CROSSCOMPILING "TRUE")
endif()

# Set default install directory permissions.
if(NOT DEFINED CMAKE_OBJDUMP)
  set(CMAKE_OBJDUMP "/opt/xpack/xpack-arm-none-eabi-gcc-14.2.1-1.1/bin/arm-none-eabi-objdump")
endif()

if(CMAKE_INSTALL_COMPONENT STREQUAL "Unspecified" OR NOT CMAKE_INSTALL_COMPONENT)
  file(INSTALL DESTINATION "${CMAKE_INSTALL_PREFIX}/include/mbedtls" TYPE FILE PERMISSIONS OWNER_READ OWNER_WRITE GROUP_READ WORLD_READ FILES
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/aes.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/aria.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/asn1.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/asn1write.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/base64.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/bignum.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/block_cipher.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/build_info.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/camellia.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/ccm.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/chacha20.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/chachapoly.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/check_config.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/cipher.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/cmac.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/compat-2.x.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/config_adjust_legacy_crypto.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/config_adjust_legacy_from_psa.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/config_adjust_psa_from_legacy.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/config_adjust_psa_superset_legacy.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/config_adjust_ssl.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/config_adjust_x509.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/config_psa.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/constant_time.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/ctr_drbg.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/debug.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/des.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/dhm.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/ecdh.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/ecdsa.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/ecjpake.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/ecp.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/entropy.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/error.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/gcm.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/hkdf.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/hmac_drbg.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/lms.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/mbedtls_config.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/md.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/md5.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/memory_buffer_alloc.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/net_sockets.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/nist_kw.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/oid.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/pem.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/pk.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/pkcs12.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/pkcs5.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/pkcs7.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/platform.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/platform_time.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/platform_util.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/poly1305.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/private_access.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/psa_util.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/ripemd160.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/rsa.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/sha1.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/sha256.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/sha3.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/sha512.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/ssl.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/ssl_cache.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/ssl_ciphersuites.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/ssl_cookie.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/ssl_ticket.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/threading.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/timing.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/version.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/x509.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/x509_crl.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/x509_crt.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/mbedtls/x509_csr.h"
    )
endif()

if(CMAKE_INSTALL_COMPONENT STREQUAL "Unspecified" OR NOT CMAKE_INSTALL_COMPONENT)
  file(INSTALL DESTINATION "${CMAKE_INSTALL_PREFIX}/include/psa" TYPE FILE PERMISSIONS OWNER_READ OWNER_WRITE GROUP_READ WORLD_READ FILES
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/psa/build_info.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/psa/crypto.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/psa/crypto_adjust_auto_enabled.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/psa/crypto_adjust_config_dependencies.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/psa/crypto_adjust_config_key_pair_types.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/psa/crypto_adjust_config_synonyms.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/psa/crypto_builtin_composites.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/psa/crypto_builtin_key_derivation.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/psa/crypto_builtin_primitives.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/psa/crypto_compat.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/psa/crypto_config.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/psa/crypto_driver_common.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/psa/crypto_driver_contexts_composites.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/psa/crypto_driver_contexts_key_derivation.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/psa/crypto_driver_contexts_primitives.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/psa/crypto_extra.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/psa/crypto_legacy.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/psa/crypto_platform.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/psa/crypto_se_driver.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/psa/crypto_sizes.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/psa/crypto_struct.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/psa/crypto_types.h"
    "/home/aklofas/Projects/ts-transformer/ts-transformer/vendor/mbedtls/include/psa/crypto_values.h"
    )
endif()

