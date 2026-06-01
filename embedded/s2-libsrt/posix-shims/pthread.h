/* Route libsrt's <pthread.h> to FreeRTOS-Plus-POSIX (newlib's is a non-impl
 * stub with a different pthread_t). */
#ifndef S2_SHIM_PTHREAD_H
#define S2_SHIM_PTHREAD_H
#include <FreeRTOS_POSIX.h>
#include <FreeRTOS_POSIX/pthread.h>
#endif
