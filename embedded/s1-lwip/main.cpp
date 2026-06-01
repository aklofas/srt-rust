// S1 — Task 3 milestone: a 4-byte datagram round-trips through lwIP UDP on the
// loopback netif. pthreads + golden come in Task 4; this de-risks lwIP/sys_arch.
#include <cstdio>
#include <cstdint>
#include <cstring>
extern "C" {
#include "FreeRTOS.h"
#include "task.h"
#include "lwip/tcpip.h"
#include "lwip/sockets.h"
#include "lwip/inet.h"
#include "lwip/netif.h"
#include "lwip/init.h"
}
extern "C" int _exit(int);

static volatile int g_lwip_up = 0;
static void tcpip_ready(void*) { g_lwip_up = 1; }

#define PORT 9000

static void run_task(void*) {
    tcpip_init(tcpip_ready, nullptr);
    while (!g_lwip_up) vTaskDelay(pdMS_TO_TICKS(1));
    // The loopback netif (lo, 127.0.0.1) is auto-added by lwIP init when
    // LWIP_HAVE_LOOPIF; give it a moment, then send to 127.0.0.1.
    vTaskDelay(pdMS_TO_TICKS(10));

    int rx = lwip_socket(AF_INET, SOCK_DGRAM, 0);
    struct sockaddr_in addr; memset(&addr, 0, sizeof addr);
    addr.sin_family = AF_INET;
    addr.sin_port = lwip_htons(PORT);
    addr.sin_addr.s_addr = lwip_htonl(INADDR_LOOPBACK);
    if (lwip_bind(rx, (struct sockaddr*)&addr, sizeof addr) != 0) { printf("FAIL[bind]\n"); fflush(stdout); _exit(1); }

    int tx = lwip_socket(AF_INET, SOCK_DGRAM, 0);
    const uint8_t msg[4] = {0xDE, 0xAD, 0xBE, 0xEF};
    lwip_sendto(tx, msg, sizeof msg, 0, (struct sockaddr*)&addr, sizeof addr);

    // Block on select() then recv — exercises lwIP's select path.
    fd_set rfds; FD_ZERO(&rfds); FD_SET(rx, &rfds);
    struct timeval tv; tv.tv_sec = 5; tv.tv_usec = 0;
    int sel = lwip_select(rx + 1, &rfds, nullptr, nullptr, &tv);
    if (sel <= 0) { printf("FAIL[select]: %d\n", sel); fflush(stdout); _exit(1); }

    uint8_t buf[16]; int n = lwip_recvfrom(rx, buf, sizeof buf, 0, nullptr, nullptr);
    int ok = (n == 4) && (memcmp(buf, msg, 4) == 0);
    if (ok) printf("PASS: s1_lwip_smoke (4B UDP loopback via select)\n");
    else    printf("FAIL[recv]: n=%d\n", n);
    fflush(stdout);
    _exit(ok ? 0 : 1);
}

int main() {
    xTaskCreate(run_task, "run", 2048, nullptr, 2, nullptr);
    vTaskStartScheduler();
    for (;;) {}
}

extern "C" void vApplicationMallocFailedHook(void) { printf("FAIL[malloc]\n"); _exit(1); }
extern "C" void vApplicationStackOverflowHook(TaskHandle_t, char*) { printf("FAIL[stack]\n"); _exit(1); }
