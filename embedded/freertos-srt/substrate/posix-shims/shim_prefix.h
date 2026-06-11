/* Force-included (-include) ahead of every TU when cross-building libsrt, so
 * these suppressions land BEFORE any newlib system header runs. newlib's
 * <sys/types.h> declares timer_t (line ~202, guarded by __timer_t_defined /
 * _TIMER_T_DECLARED) and FreeRTOS-Plus-POSIX's <FreeRTOS_POSIX/sys/types.h>
 * also typedefs it — conflict. Pre-defining the newlib sentinels makes
 * FreeRTOS-Plus-POSIX the sole definer (it's the threading layer libsrt binds
 * to). The pthread-type and sigval/sigevent collisions are handled by the
 * posix-shims/sys/_pthreadtypes.h and posix-shims/sys/signal.h shims. */
#ifndef FREERTOS_SRT_SHIM_PREFIX_H
#define FREERTOS_SRT_SHIM_PREFIX_H
#define __timer_t_defined 1
#define _TIMER_T_DECLARED

/* newlib and FreeRTOS-Plus-POSIX both unconditionally #define CLOCK_REALTIME /
 * CLOCKS_PER_SEC / TIMER_ABSTIME — with DIFFERENT values (newlib 1 / _CLOCKS_
 * PER_SEC_ / 4; +POSIX 0 / configTICK_RATE_HZ / 0x01). The +POSIX values are
 * the operative ones here (every consumer calls the +POSIX implementations, or
 * the --wrap=clock_gettime SysTick shim, which ignores the clock id), and they
 * already won by include order — lwIP's arch/cc.h pulls newlib's <sys/time.h>
 * before any +POSIX header, so FreeRTOS_POSIX/time.h redefined them LAST — at
 * the cost of a redefinition warning per TU that buried real diagnostics in
 * the CI build log. Make the takeover explicit instead: pull newlib's set
 * first (idempotent — newlib's headers are include-guarded), drop it, and let
 * FreeRTOS_POSIX/time.h be the sole definer. CLOCK_MONOTONIC rides along for
 * symmetry (newlib only defines it under feature-test macros). */
#include <sys/time.h>
#undef CLOCK_REALTIME
#undef CLOCK_MONOTONIC
#undef CLOCKS_PER_SEC
#undef TIMER_ABSTIME
#endif
