/* Minimal pthread thread-specific-data (TSD) for the libsrt-smoke libsrt port.
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
 * reclaimed on the rare thread teardown.
 *
 * Pre-scheduler context: libsrt touches this key from a global constructor
 * (before vTaskStartScheduler), where there is NO current task, so FreeRTOS's
 * vTaskSetThreadLocalStoragePointer(NULL,...) would assert (pxCurrentTCB==NULL).
 * Route get/set to a single global "bootstrap" slot until the scheduler runs.
 * That bootstrap thread is distinct from any FreeRTOS task, which is correct
 * TLS semantics — tasks start with their own (NULL) value. */
#include <FreeRTOS.h>
#include <task.h>
#include <errno.h>
#include <pthread.h>   /* our shim: pthread_key_t + the prototypes we define here */

#define FREERTOS_SRT_TLS_SLOT 0

static int   s_key_in_use = 0;
static void* s_bootstrap_tls = NULL; /* TLS for the pre-scheduler bootstrap ctx */

static int scheduler_running(void)
{
    return xTaskGetSchedulerState() != taskSCHEDULER_NOT_STARTED;
}

int pthread_key_create(pthread_key_t* key, void (*destructor)(void*))
{
    (void)destructor; /* not invoked — see file header */
    if (key == NULL)
        return EINVAL; /* nothing to return the key through; don't claim the slot */
    if (s_key_in_use)
        return EAGAIN; /* only one TLS slot available */
    s_key_in_use = 1;
    *key = FREERTOS_SRT_TLS_SLOT;
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
    if (scheduler_running())
        vTaskSetThreadLocalStoragePointer(NULL, FREERTOS_SRT_TLS_SLOT, (void*)value);
    else
        s_bootstrap_tls = (void*)value;
    return 0;
}

void* pthread_getspecific(pthread_key_t key)
{
    (void)key;
    if (scheduler_running())
        return pvTaskGetThreadLocalStoragePointer(NULL, FREERTOS_SRT_TLS_SLOT);
    return s_bootstrap_tls;
}
