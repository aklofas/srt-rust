#ifndef FREERTOS_CONFIG_H
#define FREERTOS_CONFIG_H

/* Minimal FreeRTOS V11.1.0 config for the Cortex-M4F (ARM_CM4F) port on
   QEMU mps2-an386. C++ exceptions want generous task stacks, so stacks are
   sized at task-create time; the heap is large to leave room for libstdc++. */

#define configUSE_PREEMPTION                    1
#define configUSE_TIME_SLICING                  1
#define configUSE_PORT_OPTIMISED_TASK_SELECTION 0
#define configCPU_CLOCK_HZ                      (25000000)   /* QEMU mps2 ~25 MHz */
#define configTICK_RATE_HZ                      (1000)
#define configMAX_PRIORITIES                    (5)
#define configMINIMAL_STACK_SIZE                (256)        /* words */
#define configTOTAL_HEAP_SIZE                   (192 * 1024)
#define configMAX_TASK_NAME_LEN                 (16)
#define configUSE_16_BIT_TICKS                  0
#define configIDLE_SHOULD_YIELD                 1
#define configUSE_MUTEXES                       1
#define configUSE_RECURSIVE_MUTEXES             0
#define configUSE_COUNTING_SEMAPHORES           0
#define configQUEUE_REGISTRY_SIZE               0
#define configUSE_TASK_NOTIFICATIONS            1
#define configTASK_NOTIFICATION_ARRAY_ENTRIES   1

/* Hooks */
#define configUSE_IDLE_HOOK                     0
#define configUSE_TICK_HOOK                     0
#define configUSE_MALLOC_FAILED_HOOK            1
#define configCHECK_FOR_STACK_OVERFLOW          2
#define configUSE_DAEMON_TASK_STARTUP_HOOK      0

/* Software timers */
#define configUSE_TIMERS                        1
#define configTIMER_TASK_PRIORITY               (3)
#define configTIMER_QUEUE_LENGTH                10
#define configTIMER_TASK_STACK_DEPTH            (256)

/* Thread-local storage — S0-critical: Task 4 stores per-task __cxa_eh_globals
   in a TLS pointer slot so concurrent exceptions don't clobber each other. */
#define configNUM_THREAD_LOCAL_STORAGE_POINTERS 1

/* Allocation */
#define configSUPPORT_STATIC_ALLOCATION         0
#define configSUPPORT_DYNAMIC_ALLOCATION        1

/* Cortex-M interrupt priority configuration */
#define configPRIO_BITS                         3
#define configLIBRARY_LOWEST_INTERRUPT_PRIORITY      (7)
#define configLIBRARY_MAX_SYSCALL_INTERRUPT_PRIORITY (5)
#define configKERNEL_INTERRUPT_PRIORITY \
    (configLIBRARY_LOWEST_INTERRUPT_PRIORITY << (8 - configPRIO_BITS))
#define configMAX_SYSCALL_INTERRUPT_PRIORITY \
    (configLIBRARY_MAX_SYSCALL_INTERRUPT_PRIORITY << (8 - configPRIO_BITS))

/* Optional API */
#define INCLUDE_vTaskPrioritySet                1
#define INCLUDE_uxTaskPriorityGet               1
#define INCLUDE_vTaskDelete                     1
#define INCLUDE_vTaskSuspend                    1
#define INCLUDE_vTaskDelayUntil                 1
#define INCLUDE_vTaskDelay                      1
#define INCLUDE_xTaskGetSchedulerState          1

#define configASSERT(x) if ((x) == 0) { taskDISABLE_INTERRUPTS(); for(;;); }

/* Map the FreeRTOS port handlers onto the names our vector table uses. The
   ARM_CM4F port already exports vPortSVCHandler/xPortPendSVHandler/
   xPortSysTickHandler, and startup.c references those directly, so no remap
   is needed — but if a CMSIS-style build expected SVC_Handler etc., this is
   where the #define would go. */

#endif /* FREERTOS_CONFIG_H */
