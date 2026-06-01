#include <cstdio>
#include <cstdint>
extern "C" {
#include "FreeRTOS.h"
#include "task.h"
}
extern "C" int _exit(int);

struct TaggedError { int id; };

static const int N = 3;
static const int M = 2000;
static volatile int g_fail = 0;
static volatile int g_done = 0;

static void exc_task(void* arg) {
    int my_id = (int)(intptr_t)arg;
    for (int i = 0; i < M; i++) {
        try { throw TaggedError{my_id}; }
        catch (const TaggedError& e) { if (e.id != my_id) g_fail = 1; }
        taskYIELD();   // force interleaving so concurrent throws overlap
    }
    __atomic_add_fetch(&g_done, 1, __ATOMIC_SEQ_CST);
    vTaskDelete(nullptr);
}

static void monitor_task(void*) {
    while (__atomic_load_n(&g_done, __ATOMIC_SEQ_CST) < N) vTaskDelay(1);
    if (g_fail) printf("FAIL[s0_cpp_gate]: cross-task exception corruption\n");
    else        printf("PASS: s0_cpp_gate (%d threads, %d iters, no corruption)\n", N, M);
    fflush(stdout);
    _exit(g_fail ? 1 : 0);
}

int main() {
    for (int i = 0; i < N; i++)
        xTaskCreate(exc_task, "exc", 1024, (void*)(intptr_t)i, 2, nullptr);
    xTaskCreate(monitor_task, "mon", 512, nullptr, 1, nullptr);
    vTaskStartScheduler();
    for (;;) {}
}
extern "C" void vApplicationMallocFailedHook(void) { printf("FAIL[malloc]\n"); _exit(1); }
extern "C" void vApplicationStackOverflowHook(TaskHandle_t, char*) { printf("FAIL[stack]\n"); _exit(1); }
