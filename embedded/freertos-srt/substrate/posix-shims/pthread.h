/* Route libsrt's <pthread.h> to FreeRTOS-Plus-POSIX (newlib's is a non-impl
 * stub with a different pthread_t). */
#ifndef FREERTOS_SRT_SHIM_PTHREAD_H
#define FREERTOS_SRT_SHIM_PTHREAD_H
#include <FreeRTOS_POSIX.h>
#include <FreeRTOS_POSIX/pthread.h>

/* FreeRTOS-Plus-POSIX provides NO thread-specific-data (pthread_key_*) API, but
 * libsrt's sync_posix.cpp uses one key for its per-thread last-error store.
 * Declare the API here; pthread_key_shim.c implements it over FreeRTOS's single
 * thread-local-storage slot (configNUM_THREAD_LOCAL_STORAGE_POINTERS=1). The
 * definitions link into the firmware, not libsrt.a. */
#ifndef FREERTOS_SRT_PTHREAD_KEY_DECLARED
#define FREERTOS_SRT_PTHREAD_KEY_DECLARED
typedef int pthread_key_t;
#ifdef __cplusplus
extern "C" {
#endif
int   pthread_key_create(pthread_key_t* key, void (*destructor)(void*));
int   pthread_key_delete(pthread_key_t key);
int   pthread_setspecific(pthread_key_t key, const void* value);
void* pthread_getspecific(pthread_key_t key);
#ifdef __cplusplus
}
#endif
#endif /* FREERTOS_SRT_PTHREAD_KEY_DECLARED */

#endif
