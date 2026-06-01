/* lwIP arch shim for arm-none-eabi + newlib. lwIP pulls fixed-width types and
 * printf formatters from the C library (LWIP_NO_STDINT_H / LWIP_NO_INTTYPES_H
 * default 0 → <stdint.h>/<inttypes.h>); we only need the diagnostic/abort
 * hooks and a randomness source. */
#ifndef LWIP_ARCH_CC_H
#define LWIP_ARCH_CC_H

#include <stdio.h>
#include <stdlib.h>
/* LWIP_TIMEVAL_PRIVATE=0 (lwipopts.h) → every lwIP TU must see the system
 * struct timeval; lwIP's docs require including <sys/time.h> from cc.h. */
#include <sys/time.h>

#define LWIP_PLATFORM_DIAG(x)   do { printf x; } while (0)
#define LWIP_PLATFORM_ASSERT(x) do { printf("LWIP ASSERT: %s\n", x); abort(); } while (0)

#endif /* LWIP_ARCH_CC_H */
