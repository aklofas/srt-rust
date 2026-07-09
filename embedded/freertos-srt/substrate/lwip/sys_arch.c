/* lwIP NO_SYS=0 sys_arch port onto FreeRTOS. Maps lwIP's semaphores/mutexes/
 * mailboxes/threads onto FreeRTOS primitives. Mailboxes are FreeRTOS queues of
 * void* (lwIP posts pbuf/api-msg pointers). Standard reference shape (cf.
 * lwip-contrib ports/freertos), trimmed to what loopback UDP + sockets need. */
#include "lwip/opt.h"
#include "lwip/sys.h"
#include "lwip/err.h"
#include "arch/sys_arch.h"
#include "diag.h"

extern __attribute__((noreturn)) void _exit(int);

#include "FreeRTOS.h"
#include "task.h"
#include "queue.h"
#include "semphr.h"

/* ---- protection (SYS_LIGHTWEIGHT_PROT) ---- */
sys_prot_t sys_arch_protect(void)        { taskENTER_CRITICAL(); return 0; }
void       sys_arch_unprotect(sys_prot_t p) { (void)p; taskEXIT_CRITICAL(); }

void sys_init(void) {}

u32_t sys_now(void) { return (u32_t)(xTaskGetTickCount() * portTICK_PERIOD_MS); }

/* ---- semaphores (binary/counting) ---- */
err_t sys_sem_new(sys_sem_t *sem, u8_t count) {
    sem->sem = xSemaphoreCreateBinary();
    if (sem->sem == NULL) return ERR_MEM;
    if (count > 0) xSemaphoreGive(sem->sem);
    return ERR_OK;
}
void sys_sem_free(sys_sem_t *sem)   { vSemaphoreDelete(sem->sem); sem->sem = NULL; }
void sys_sem_signal(sys_sem_t *sem) { xSemaphoreGive(sem->sem); }
int  sys_sem_valid(sys_sem_t *sem)  { return sem->sem != NULL; }
void sys_sem_set_invalid(sys_sem_t *sem) { sem->sem = NULL; }

u32_t sys_arch_sem_wait(sys_sem_t *sem, u32_t timeout_ms) {
    TickType_t start = xTaskGetTickCount();
    TickType_t to = (timeout_ms == 0) ? portMAX_DELAY : pdMS_TO_TICKS(timeout_ms);
    if (xSemaphoreTake(sem->sem, to) == pdTRUE) {
        return (u32_t)((xTaskGetTickCount() - start) * portTICK_PERIOD_MS);
    }
    return SYS_ARCH_TIMEOUT;
}

/* ---- mutexes ---- */
err_t sys_mutex_new(sys_mutex_t *mtx) {
    mtx->mtx = xSemaphoreCreateMutex();
    return mtx->mtx == NULL ? ERR_MEM : ERR_OK;
}
void sys_mutex_free(sys_mutex_t *mtx)   { vSemaphoreDelete(mtx->mtx); mtx->mtx = NULL; }
void sys_mutex_lock(sys_mutex_t *mtx)   { xSemaphoreTake(mtx->mtx, portMAX_DELAY); }
void sys_mutex_unlock(sys_mutex_t *mtx) { xSemaphoreGive(mtx->mtx); }
int  sys_mutex_valid(sys_mutex_t *mtx)  { return mtx->mtx != NULL; }
void sys_mutex_set_invalid(sys_mutex_t *mtx) { mtx->mtx = NULL; }

/* ---- mailboxes (queue of void*) ---- */
err_t sys_mbox_new(sys_mbox_t *mbox, int size) {
    mbox->mbox = xQueueCreate((UBaseType_t)size, sizeof(void *));
    return mbox->mbox == NULL ? ERR_MEM : ERR_OK;
}
void sys_mbox_free(sys_mbox_t *mbox)  { vQueueDelete(mbox->mbox); mbox->mbox = NULL; }
int  sys_mbox_valid(sys_mbox_t *mbox) { return mbox->mbox != NULL; }
void sys_mbox_set_invalid(sys_mbox_t *mbox) { mbox->mbox = NULL; }

void sys_mbox_post(sys_mbox_t *mbox, void *msg) {
    xQueueSendToBack(mbox->mbox, &msg, portMAX_DELAY);
}
err_t sys_mbox_trypost(sys_mbox_t *mbox, void *msg) {
    return xQueueSendToBack(mbox->mbox, &msg, 0) == pdTRUE ? ERR_OK : ERR_MEM;
}
err_t sys_mbox_trypost_fromisr(sys_mbox_t *mbox, void *msg) {
    BaseType_t woken = pdFALSE;
    BaseType_t r = xQueueSendToBackFromISR(mbox->mbox, &msg, &woken);
    portYIELD_FROM_ISR(woken);
    return r == pdTRUE ? ERR_OK : ERR_MEM;
}

u32_t sys_arch_mbox_fetch(sys_mbox_t *mbox, void **msg, u32_t timeout_ms) {
    void *dummy; if (msg == NULL) msg = &dummy;
    TickType_t start = xTaskGetTickCount();
    TickType_t to = (timeout_ms == 0) ? portMAX_DELAY : pdMS_TO_TICKS(timeout_ms);
    if (xQueueReceive(mbox->mbox, &(*msg), to) == pdTRUE) {
        return (u32_t)((xTaskGetTickCount() - start) * portTICK_PERIOD_MS);
    }
    *msg = NULL;
    return SYS_ARCH_TIMEOUT;
}
u32_t sys_arch_mbox_tryfetch(sys_mbox_t *mbox, void **msg) {
    void *dummy; if (msg == NULL) msg = &dummy;
    return xQueueReceive(mbox->mbox, &(*msg), 0) == pdTRUE ? 0 : SYS_MBOX_EMPTY;
}

/* ---- threads ---- */
sys_thread_t sys_thread_new(const char *name, lwip_thread_fn fn, void *arg,
                            int stacksize, int prio) {
    TaskHandle_t h = NULL;
    /* lwIP stacksize is in bytes; FreeRTOS xTaskCreate depth is in words.
     * Round up so a non-word-aligned byte size doesn't under-allocate. */
    if (xTaskCreate((TaskFunction_t)fn, name,
                    (configSTACK_DEPTH_TYPE)(((size_t)stacksize + sizeof(StackType_t) - 1) / sizeof(StackType_t)),
                    arg, (UBaseType_t)prio, &h) != pdPASS) {
        tst_diag_write0("FAIL[sys_thread_new] ");
        tst_diag_write0(name);
        tst_diag_write0("\n");
        _exit(1);
    }
    return h;
}
