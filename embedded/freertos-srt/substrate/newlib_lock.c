/* newlib retargetable-locking backend for FreeRTOS (EMB-HEAP-1).
 *
 * The xpack arm-none-eabi newlib is built with _RETARGETABLE_LOCKING: libc.a
 * references __retarget_lock_* and links dummy no-op fallbacks. Under a
 * preemptive scheduler those no-ops leave malloc/free/stdio/env completely
 * unsynchronized. These strong definitions override the archive dummies.
 *
 * Pre-scheduler, everything is single-context: acquire/release are no-ops
 * (mirrors pthread_key_shim.c's scheduler_running() pattern). The static
 * locks are created lazily inside a suspend-all window so a post-scheduler
 * first-use cannot race. */
#include <stdlib.h>
#include <sys/lock.h>
#include "FreeRTOS.h"
#include "semphr.h"
#include "task.h"
#include "diag.h"

struct __lock {
    SemaphoreHandle_t h;
    StaticSemaphore_t buf;
};

struct __lock __lock___sinit_recursive_mutex;
struct __lock __lock___sfp_recursive_mutex;
struct __lock __lock___atexit_recursive_mutex;
struct __lock __lock___at_quick_exit_mutex;
struct __lock __lock___malloc_recursive_mutex;
struct __lock __lock___env_recursive_mutex;
struct __lock __lock___tz_mutex;
struct __lock __lock___dd_hash_mutex;
struct __lock __lock___arc4random_mutex;

static int scheduler_running(void) {
    return xTaskGetSchedulerState() != taskSCHEDULER_NOT_STARTED;
}

/* All newlib-internal locks are taken recursively-safe; backing every lock
 * with a recursive mutex is a superset of the required semantics. */
static void ensure(struct __lock *l) {
    if (l->h) return;
    vTaskSuspendAll();
    if (!l->h) l->h = xSemaphoreCreateRecursiveMutexStatic(&l->buf);
    (void)xTaskResumeAll();
}

void __retarget_lock_init(_LOCK_T *lock) { __retarget_lock_init_recursive(lock); }

void __retarget_lock_init_recursive(_LOCK_T *lock) {
    struct __lock *l = calloc(1, sizeof *l);
    if (!l) tst_diag_fail("lock_alloc");
    *lock = l;
}

void __retarget_lock_close(_LOCK_T lock) { __retarget_lock_close_recursive(lock); }

void __retarget_lock_close_recursive(_LOCK_T lock) {
    if (lock && lock->h) vSemaphoreDelete(lock->h);
    free(lock);
}

void __retarget_lock_acquire(_LOCK_T lock) { __retarget_lock_acquire_recursive(lock); }

void __retarget_lock_acquire_recursive(_LOCK_T lock) {
    if (!scheduler_running()) return;
    ensure(lock);
    (void)xSemaphoreTakeRecursive(lock->h, portMAX_DELAY);
}

int __retarget_lock_try_acquire(_LOCK_T lock) { return __retarget_lock_try_acquire_recursive(lock); }

int __retarget_lock_try_acquire_recursive(_LOCK_T lock) {
    if (!scheduler_running()) return 1;
    ensure(lock);
    return xSemaphoreTakeRecursive(lock->h, 0) == pdTRUE;
}

void __retarget_lock_release(_LOCK_T lock) { __retarget_lock_release_recursive(lock); }

void __retarget_lock_release_recursive(_LOCK_T lock) {
    if (!scheduler_running()) return;
    (void)xSemaphoreGiveRecursive(lock->h);
}

/* Heap-morecore guard for startup.c's _sbrk (belt-and-suspenders: malloc's
 * own lock already serializes the malloc->_sbrk path). */
void tst_heap_lock(void)   { if (scheduler_running()) vTaskSuspendAll(); }
void tst_heap_unlock(void) { if (scheduler_running()) (void)xTaskResumeAll(); }
