// fault-smoke — proves the fatal-path diagnostics: a deliberate fault must
// produce a labeled FAIL token + fast nonzero semihosting exit, not a silent
// 60 s hang. The GATE asserts the FAIL token (inverted expectation).
#include <cstdio>
#include <cstdlib>
extern "C" {
#include "FreeRTOS.h"
#include "task.h"
}

extern "C" void _exit(int) __attribute__((noreturn));

static void crash_task(void*) {
    printf("fault-smoke: about to udf\n");
    fflush(stdout);
    __builtin_trap();       // udf -> UsageFault, escalates to HardFault (USGFAULTENA off)
    for (;;) {}
}

int main() {
    /* Check task creation explicitly: the malloc-failed hook already makes a
     * heap exhaustion loud, but a direct check here gives clearer triage and
     * is consistent with the sibling firmwares (malloc-stress, loopback-arq). */
    if (xTaskCreate(crash_task, "crash", 1024, nullptr, 2, nullptr) != pdPASS) {
        printf("FAIL[fault_smoke]: xTaskCreate\n"); fflush(stdout); _exit(1);
    }
    vTaskStartScheduler();
    for (;;) {}
}

extern "C" void vApplicationMallocFailedHook(void) { printf("FAIL[malloc]\n"); _exit(1); }
extern "C" void vApplicationStackOverflowHook(TaskHandle_t, char*) { printf("FAIL[stack]\n"); _exit(1); }
