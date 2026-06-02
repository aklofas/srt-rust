/* Intentionally (almost) empty. libsrt's CMake forces -D_GNU_SOURCE, which bumps
 * newlib's __POSIX_VISIBLE to 200809 (>=199506) and so activates newlib's real
 * <sys/_pthreadtypes.h> (gated at its line 21). That declares pthread_t /
 * pthread_mutex_t / pthread_cond_t / timer_t as NEWLIB types, conflicting with
 * the FreeRTOS-Plus-POSIX typedefs libsrt actually uses (via our pthread.h
 * shim). S1 never defined _GNU_SOURCE, so it never hit this. Pre-defining the
 * newlib include guard suppresses newlib's version; the threading types come
 * exclusively from FreeRTOS-Plus-POSIX. */
#ifndef _SYS__PTHREADTYPES_H_
#define _SYS__PTHREADTYPES_H_
#endif
