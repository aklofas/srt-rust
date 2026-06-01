#include <cstdio>
#include <cstdint>
extern "C" {
#include "FreeRTOS.h"
#include "task.h"
#include "FreeRTOS_POSIX.h"
#include "FreeRTOS_POSIX/pthread.h"
}
extern "C" int _exit(int);

// Task 5: re-run the concurrent C++ exception gate with the worker threads
// created as FreeRTOS-Plus-POSIX *pthreads* — the threading API libsrt's
// sync_posix.cpp binds to — to validate that the per-task __cxa_eh_globals
// override (cxa_override.cpp) still isolates exception state when threads come
// in through the POSIX wrapper (which maps each pthread onto a FreeRTOS task
// via xTaskCreate + a per-task application tag).

struct TaggedError { int id; };

static const int N = 3;
static const int M = 2000;
static volatile int g_fail = 0;

// Each pthread throws ITS OWN id and must catch exactly that — any cross-thread
// bleed of exception state shows up as e.id != my_id. taskYIELD() between
// iterations forces the throws of the 3 threads to interleave mid-flight.
static void* exc_thread(void* arg) {
    int my_id = (int)(intptr_t)arg;
    for (int i = 0; i < M; i++) {
        try { throw TaggedError{my_id}; }
        catch (const TaggedError& e) { if (e.id != my_id) g_fail = 1; }
        taskYIELD();
    }
    return nullptr;
}

// pthread_create must run from a FreeRTOS task context (the POSIX layer calls
// FreeRTOS task APIs under the hood), so the scheduler is started with this
// single bootstrap task that spawns + joins the workers, then reports.
static void bootstrap_task(void*) {
    pthread_t th[N];
    // The default pthread stack (PTHREAD_STACK_MIN = configMINIMAL_STACK_SIZE *
    // sizeof(StackType_t) = 1 KiB) is far too small for the C++ exception
    // unwinder. Match the 4 KiB (1024-word) stack the Task 4 raw xTaskCreate
    // workers used so the two-phase unwind has room.
    pthread_attr_t attr;
    pthread_attr_init(&attr);
    pthread_attr_setstacksize(&attr, 4096);
    for (int i = 0; i < N; i++) {
        if (pthread_create(&th[i], &attr, exc_thread, (void*)(intptr_t)i) != 0) {
            printf("FAIL[s0_cpp_gate]: pthread_create failed\n");
            fflush(stdout);
            _exit(1);
        }
    }
    for (int i = 0; i < N; i++) pthread_join(th[i], nullptr);

    if (g_fail) printf("FAIL[s0_cpp_gate]: cross-thread exception corruption\n");
    else        printf("PASS: s0_cpp_gate (%d threads, %d iters, no corruption)\n", N, M);
    fflush(stdout);
    _exit(g_fail ? 1 : 0);
}

int main() {
    xTaskCreate(bootstrap_task, "boot", 1024, nullptr, 2, nullptr);
    vTaskStartScheduler();
    for (;;) {}
}
extern "C" void vApplicationMallocFailedHook(void) { printf("FAIL[malloc]\n"); _exit(1); }
extern "C" void vApplicationStackOverflowHook(TaskHandle_t, char*) { printf("FAIL[stack]\n"); _exit(1); }
