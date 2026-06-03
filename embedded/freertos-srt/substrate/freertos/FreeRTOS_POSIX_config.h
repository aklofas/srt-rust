/* FreeRTOS+POSIX knob overrides for the exceptions C++ exceptions gate.
 *
 * The wrapper pulls all posixconfig* defaults from
 * FreeRTOS_POSIX_portable_default.h; the only port-specific deviations we need
 * (the newlib typedef-collision suppressions) live in FreeRTOS_POSIX_portable.h.
 * This file exists as the documented override point — currently the defaults
 * suffice, so it is intentionally empty of overrides. */

#ifndef _FREERTOS_POSIX_CONFIG_H_
#define _FREERTOS_POSIX_CONFIG_H_

/* (no overrides — defaults from FreeRTOS_POSIX_portable_default.h are fine) */

#endif /* _FREERTOS_POSIX_CONFIG_H_ */
