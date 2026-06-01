#include <cstdio>
extern "C" {
#include "FreeRTOS.h"
#include "task.h"
}
extern "C" __attribute__((noreturn)) void _exit(int);

/* Task 3: prove the full libstdc++ + ARM unwinder works inside a single
   FreeRTOS task. A throw/catch that crosses no task boundary still exercises
   __cxa_throw, the .ARM.exidx/.extab unwind tables, and __cxa_get_globals
   (default per-process EH state — single-task, so no TLS override needed yet). */
struct TaggedError { int id; };

static void exc_task(void*) {
    int caught = -1;
    try {
        throw TaggedError{42};
    } catch (const TaggedError& e) {
        caught = e.id;
    }
    if (caught == 42) printf("PASS: single_task_exception\n");
    else              printf("FAIL[single]: caught=%d\n", caught);
    fflush(stdout);
    _exit(caught == 42 ? 0 : 1);
}

int main() {
    /* Generous stack: stack unwinding + libstdc++ EH machinery is hungry. */
    xTaskCreate(exc_task, "exc", 1024, nullptr, 2, nullptr);
    vTaskStartScheduler();
    for (;;) {}
}

extern "C" void vApplicationMallocFailedHook(void) {
    printf("FAIL[malloc]\n"); fflush(stdout); _exit(1);
}
extern "C" void vApplicationStackOverflowHook(TaskHandle_t, char*) {
    printf("FAIL[stack]\n"); fflush(stdout); _exit(1);
}
