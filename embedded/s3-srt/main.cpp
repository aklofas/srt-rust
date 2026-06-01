// S3 — Task 1 milestone: the S2 substrate + the lossy loopback netif boot in
// the new dir (the SRT app comes in Task 2). Confirms the copy + lwipopts delta
// + lossy_netif compile and bring up lwIP.
#include <cstdio>
extern "C" {
#include "FreeRTOS.h"
#include "task.h"
#include "lwip/tcpip.h"
#include "lossy_netif.h"
}
extern "C" int _exit(int);

static volatile int g_up = 0;
static void tcpip_ready(void*) { g_up = 1; }

static void run_task(void*) {
    tcpip_init(tcpip_ready, nullptr);
    while (!g_up) vTaskDelay(pdMS_TO_TICKS(1));
    lossy_netif_up();
    printf("BOOT: s3_srt substrate + lossy netif up\n");
    fflush(stdout);
    _exit(0);
}

int main() {
    xTaskCreate(run_task, "run", 2048, nullptr, 2, nullptr);
    vTaskStartScheduler();
    for (;;) {}
}

extern "C" void vApplicationMallocFailedHook(void) { printf("FAIL[malloc]\n"); _exit(1); }
extern "C" void vApplicationStackOverflowHook(TaskHandle_t, char*) { printf("FAIL[stack]\n"); _exit(1); }
