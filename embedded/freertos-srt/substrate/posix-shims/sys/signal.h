/* newlib's <sys/signal.h> defines `union sigval` / `struct sigevent` /
 * `siginfo_t` under its 199309 visibility gate, with NO sub-sentinel. So does
 * FreeRTOS-Plus-POSIX's <FreeRTOS_POSIX/signal.h> (its timer needs the complete,
 * pthread-style sigevent), with a different guard. When a libsrt TU pulls both
 * (newlib via platform_sys.h, FreeRTOS via our pthread.h shim) they collide on
 * redefinition. Mask newlib's 199309 block by lowering __POSIX_VISIBLE only
 * across the real header, leaving FreeRTOS's as the single definition. The basic
 * signal API (SIG*, signal(), raise(), sig_atomic_t) sits below the 199309 gate
 * and is unaffected. */
#ifndef FREERTOS_SRT_SHIM_SYS_SIGNAL_H
#define FREERTOS_SRT_SHIM_SYS_SIGNAL_H
#include <sys/features.h>   /* pin __POSIX_VISIBLE before we override it */
#pragma push_macro("__POSIX_VISIBLE")
#undef __POSIX_VISIBLE
#define __POSIX_VISIBLE 199209
#include_next <sys/signal.h>
#pragma pop_macro("__POSIX_VISIBLE")
#endif
