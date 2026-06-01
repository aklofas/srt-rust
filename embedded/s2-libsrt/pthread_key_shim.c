/* Minimal pthread thread-specific-data (TSD) for the S2 libsrt port.
 *
 * FreeRTOS-Plus-POSIX ships no pthread_key_* API, but libsrt's sync_posix.cpp
 * needs exactly ONE key (CThreadError's per-thread last-error). We back that key
 * with FreeRTOS's single thread-local-storage pointer slot 0
 * (configNUM_THREAD_LOCAL_STORAGE_POINTERS=1). Only one live key is supported,
 * which matches libsrt's usage.
 *
 * Limitation: the key destructor is NOT invoked on task exit (FreeRTOS TLS has
 * no per-task delete callback unless configTHREAD_LOCAL_STORAGE_DELETE_CALLBACKS
 * is enabled). That is acceptable here — libsrt's GC thread is long-lived and a
 * bare-metal target never exits; the small per-thread CUDTException simply isn't
 * reclaimed on the rare thread teardown. */
#include <FreeRTOS.h>
#include <task.h>
#include <errno.h>
#include <pthread.h>   /* our shim: pthread_key_t + the prototypes we define here */

#define S2_TLS_SLOT 0

static int s_key_in_use = 0;

int pthread_key_create(pthread_key_t* key, void (*destructor)(void*))
{
    (void)destructor; /* not invoked — see file header */
    if (s_key_in_use)
        return EAGAIN; /* only one TLS slot available */
    s_key_in_use = 1;
    if (key)
        *key = S2_TLS_SLOT;
    return 0;
}

int pthread_key_delete(pthread_key_t key)
{
    (void)key;
    s_key_in_use = 0;
    return 0;
}

int pthread_setspecific(pthread_key_t key, const void* value)
{
    (void)key;
    vTaskSetThreadLocalStoragePointer(NULL, S2_TLS_SLOT, (void*)value);
    return 0;
}

void* pthread_getspecific(pthread_key_t key)
{
    (void)key;
    return pvTaskGetThreadLocalStoragePointer(NULL, S2_TLS_SLOT);
}
