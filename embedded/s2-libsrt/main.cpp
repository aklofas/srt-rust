// S2 — Task 1 milestone: the S1 substrate boots in the new dir (libsrt comes
// in Task 3). Confirms the copy + lwipopts delta didn't break the substrate.
#include <cstdio>
#include <cstdint>
extern "C" {
#include "FreeRTOS.h"
#include "task.h"
}
extern "C" int _exit(int);

static void boot_task(void*) {
    printf("BOOT: s2_libsrt substrate running\n");
    fflush(stdout);
    _exit(0);
}

int main() {
    xTaskCreate(boot_task, "boot", 1024, nullptr, 2, nullptr);
    vTaskStartScheduler();
    for (;;) {}
}

extern "C" void vApplicationMallocFailedHook(void) { printf("FAIL[malloc]\n"); _exit(1); }
extern "C" void vApplicationStackOverflowHook(TaskHandle_t, char*) { printf("FAIL[stack]\n"); _exit(1); }
