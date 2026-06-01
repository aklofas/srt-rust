/* Port-specific FreeRTOS+POSIX configuration for the S0 C++ exceptions gate
 * (arm-none-eabi-gcc / newlib, Cortex-M4F, QEMU mps2-an386).
 *
 * FreeRTOS_POSIX.h includes this header first. Its job is to suppress the
 * FreeRTOS+POSIX typedefs that the arm-none-eabi newlib headers ALSO define,
 * so the two don't collide. Modeled on the vendored STM32+GNU port header
 * (portable/st/stm32h745zi_nucleo/FreeRTOS_POSIX_portable.h) — same newlib
 * lineage, same collisions. */

#ifndef _FREERTOS_POSIX_PORTABLE_H_
#define _FREERTOS_POSIX_PORTABLE_H_

/* newlib already defines these via <sys/_timespec.h> / <sys/timespec.h>. */
#define posixconfigENABLE_TIMESPEC      0
#include <sys/_timespec.h>

#define posixconfigENABLE_ITIMERSPEC    0
#include <sys/timespec.h>

/* newlib already provides mode_t and clockid_t. */
#define posixconfigENABLE_MODE_T        0
#define posixconfigENABLE_CLOCKID_T     0

/* Suppress newlib timer_t so it doesn't fight the FreeRTOS+POSIX one. */
#define __timer_t_defined               1

/* Block newlib's <sys/_pthreadtypes.h> definitions (which differ from the
 * FreeRTOS+POSIX pthread_* typedefs) before pulling <sys/types.h>. */
#define _SYS__PTHREADTYPES_H_
#include <sys/types.h>

/* Use newlib's sched_param/sched_yield/etc. rather than the POSIX-layer copy. */
#define posixconfigENABLE_SCHED_PARAM   0
#include <sys/sched.h>

#endif /* _FREERTOS_POSIX_PORTABLE_H_ */
