#include <cstdio>
extern "C" {
#include "FreeRTOS.h"
#include "task.h"
}
extern "C" __attribute__((noreturn)) void _exit(int);

static void hello_task(void* arg) {
    int id = (int)(long)arg;
    for (int i = 0; i < 3; i++) {
        printf("task%d tick %d\n", id, i);
        fflush(stdout);
        vTaskDelay(1);
    }
    if (id == 1) {
        printf("PASS: freertos_hello\n");
        fflush(stdout);
        _exit(0);
    }
    vTaskDelete(nullptr);
}

int main() {
    xTaskCreate(hello_task, "t0", 512, (void*)0, 2, nullptr);
    xTaskCreate(hello_task, "t1", 512, (void*)1, 2, nullptr);
    vTaskStartScheduler();
    for (;;) {}
}

extern "C" void vApplicationMallocFailedHook(void) {
    printf("FAIL[malloc]\n"); fflush(stdout); _exit(1);
}
extern "C" void vApplicationStackOverflowHook(TaskHandle_t, char*) {
    printf("FAIL[stack]\n"); fflush(stdout); _exit(1);
}
