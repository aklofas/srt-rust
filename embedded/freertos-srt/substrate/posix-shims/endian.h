/* Bare-metal newlib ships no <endian.h>. libsrt's utilities.h includes it on the
 * __linux__/__CYGWIN__/__GNU__/__GLIBC__ branch (we select it with -D__GNU__, the
 * only non-Linux option that doesn't also trigger epoll elsewhere) and otherwise
 * hits an unconditional `#error Endian: platform not supported`. Provide the full
 * glibc-style byte-order macro set for little-endian ARM (Cortex-M is LE; GCC
 * confirms __BYTE_ORDER__ == __ORDER_LITTLE_ENDIAN__). Guarded so a later
 * <machine/endian.h> can't double-define. */
#ifndef S2_SHIM_ENDIAN_H
#define S2_SHIM_ENDIAN_H

#include <stdint.h>

#ifndef __LITTLE_ENDIAN
#define __LITTLE_ENDIAN 1234
#endif
#ifndef __BIG_ENDIAN
#define __BIG_ENDIAN    4321
#endif
#ifndef __PDP_ENDIAN
#define __PDP_ENDIAN    3412
#endif
#ifndef __BYTE_ORDER
#define __BYTE_ORDER    __LITTLE_ENDIAN
#endif

/* Little-endian host: host<->little is identity, host<->big is a byte swap. */
#ifndef htole16
#define htole16(x) ((uint16_t)(x))
#endif
#ifndef le16toh
#define le16toh(x) ((uint16_t)(x))
#endif
#ifndef htobe16
#define htobe16(x) __builtin_bswap16((uint16_t)(x))
#endif
#ifndef be16toh
#define be16toh(x) __builtin_bswap16((uint16_t)(x))
#endif

#ifndef htole32
#define htole32(x) ((uint32_t)(x))
#endif
#ifndef le32toh
#define le32toh(x) ((uint32_t)(x))
#endif
#ifndef htobe32
#define htobe32(x) __builtin_bswap32((uint32_t)(x))
#endif
#ifndef be32toh
#define be32toh(x) __builtin_bswap32((uint32_t)(x))
#endif

#ifndef htole64
#define htole64(x) ((uint64_t)(x))
#endif
#ifndef le64toh
#define le64toh(x) ((uint64_t)(x))
#endif
#ifndef htobe64
#define htobe64(x) __builtin_bswap64((uint64_t)(x))
#endif
#ifndef be64toh
#define be64toh(x) __builtin_bswap64((uint64_t)(x))
#endif

#endif
