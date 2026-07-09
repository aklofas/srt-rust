#ifndef LWIP_ARCH_SYS_ARCH_H
#define LWIP_ARCH_SYS_ARCH_H

#include "FreeRTOS.h"
#include "task.h"
#include "queue.h"
#include "semphr.h"

#define SYS_MBOX_NULL  NULL
#define SYS_SEM_NULL   NULL

typedef UBaseType_t sys_prot_t;
typedef struct { SemaphoreHandle_t sem; } sys_sem_t;
typedef struct { SemaphoreHandle_t mtx; } sys_mutex_t;
typedef struct { QueueHandle_t     mbox; } sys_mbox_t;
typedef TaskHandle_t sys_thread_t;

#endif /* LWIP_ARCH_SYS_ARCH_H */
