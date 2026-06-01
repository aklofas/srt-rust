/* Force-included (-include) ahead of every TU when cross-building libsrt, so
 * these suppressions land BEFORE any newlib system header runs. newlib's
 * <sys/types.h> declares timer_t (line ~202, guarded by __timer_t_defined /
 * _TIMER_T_DECLARED) and FreeRTOS-Plus-POSIX's <FreeRTOS_POSIX/sys/types.h>
 * also typedefs it — conflict. Pre-defining the newlib sentinels makes
 * FreeRTOS-Plus-POSIX the sole definer (it's the threading layer libsrt binds
 * to). The pthread-type and sigval/sigevent collisions are handled by the
 * posix-shims/sys/_pthreadtypes.h and posix-shims/sys/signal.h shims. */
#ifndef S2_PREFIX_H
#define S2_PREFIX_H
#define __timer_t_defined 1
#define _TIMER_T_DECLARED
#endif
