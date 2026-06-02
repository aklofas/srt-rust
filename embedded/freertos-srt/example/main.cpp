// S4 — SRT caller (LIVE mode) on bare metal streaming the 564-byte golden out a
// real lan9118 NIC over QEMU SLIRP user-net to a host tst-srt listener. LIVE
// (not FILE) so it interoperates with the host tst-srt LIVE-streaming listener;
// see srt_opts.h. Over lossless SLIRP the golden still arrives byte-exact. The
// firmware only proves it SENT; the host is the authoritative byte-exact
// verifier (the bytes leave the chip). Two phases: plain + AES-128 (S4_PASSPHRASE).
#include <cstdio>
#include <cstdint>
#include <cstring>
#include <srt/srt.h>                 // C++ linkage (pulls C++ stdlib)
extern "C" {
#include "FreeRTOS.h"
#include "task.h"
#include "FreeRTOS_POSIX.h"
#include "FreeRTOS_POSIX/pthread.h"
#include "lwip/tcpip.h"
#include "lwip/inet.h"
#include "lan9118_netif.h"
}
#include "golden.h"                  // GOLDEN[], GOLDEN_LEN = 564
#include "srt_opts.h"

#define PORT   9000
#define REPEAT 64
static const int STREAM_LEN = (int)GOLDEN_LEN * REPEAT;   // 36096 bytes

#ifdef SRT_PASSPHRASE
#define S4_TAG "s4_aes"
#define S4_ENC " (AES-128)"
#else
#define S4_TAG "s4_plain"
#define S4_ENC ""
#endif

static volatile int g_up = 0;
static volatile int g_fail = 0;
static const char*  g_where = "";
static SRT_TRACEBSTATS g_tx_stats;

static void tcpip_ready(void*) { g_up = 1; }
static void fail(const char* w) { g_fail = 1; g_where = w; }

// Dedicated FreeRTOS task: poll the lan9118 RX FIFO into lwIP.
static void rx_poll_task(void*) {
    for (;;) { lan9118_poll(); vTaskDelay(1); }
}

// Caller: connect to the HOST (SLIRP gateway 10.0.2.2:PORT) and stream GOLDEN.
static void* caller_thread(void*) {
    struct sockaddr_in sa; memset(&sa, 0, sizeof sa);
    sa.sin_family = AF_INET; sa.sin_port = lwip_htons(PORT);
    sa.sin_addr.s_addr = lwip_htonl(0x0A000202);   /* 10.0.2.2 = SLIRP host alias */
    SRTSOCKET cs = srt_create_socket();
    if (cs == SRT_INVALID_SOCK || srt_apply_opts(cs) != 0) { fail("call_opts"); return nullptr; }
    if (srt_connect(cs, (sockaddr*)&sa, sizeof sa) == SRT_ERROR) { fail("connect"); return nullptr; }
    for (int r = 0; r < REPEAT && !g_fail; r++) {
        int off = 0;
        while (off < (int)GOLDEN_LEN) {
            int n = srt_send(cs, (const char*)GOLDEN + off, (int)GOLDEN_LEN - off);
            if (n <= 0) { fail("send"); break; }
            off += n;
        }
    }
    // LIVE-mode srt_close does NOT linger, and srt_send only buffers — the send
    // queue worker paces the packets onto the wire afterward. Wait for the send
    // buffer to drain (pktSndBuf == 0) so all REPEAT messages leave the NIC,
    // then a short grace so the host reads its recv buffer before we close and
    // QEMU tears the network down. Both bounded so a stall can't hang the gate.
    for (int i = 0; i < 8000; i++) {
        srt_bstats(cs, &g_tx_stats, 0);
        if (g_tx_stats.pktSndBuf == 0) break;
        vTaskDelay(pdMS_TO_TICKS(1));
    }
    vTaskDelay(pdMS_TO_TICKS(2000));
    srt_close(cs);
    return nullptr;
}

static void run_task(void*) {
    tcpip_init(tcpip_ready, nullptr);
    while (!g_up) vTaskDelay(pdMS_TO_TICKS(1));
    lan9118_netif_up(10,0,2,15, 10,0,2,2);             // guest 10.0.2.15, gw 10.0.2.2
    // rx poll task at priority 3: below tcpip (4) so pushed frames are processed
    // promptly, above run_task (2). NOTE configMAX_PRIORITIES=5 -> max prio is 4.
    xTaskCreate(rx_poll_task, "rx", 1024, nullptr, 3, nullptr);

    if (srt_startup() < 0) fail("startup");

    pthread_t ct;
    pthread_attr_t attr; pthread_attr_init(&attr);
    pthread_attr_setstacksize(&attr, 32768);
    pthread_create(&ct, &attr, caller_thread, nullptr);
    pthread_join(ct, nullptr);

    // The firmware cannot verify bytes (they left the chip); it only reports the
    // send completed. The HOST tst-srt listener is the authoritative verifier.
    int ok = !g_fail;
    if (ok) printf("PASS: " S4_TAG "_sent (GOLDEN x %d streamed%s)\n", REPEAT, S4_ENC);
    else    printf("FAIL[" S4_TAG "]: where=%s\n", g_where);
    fflush(stdout);
    srt_cleanup();
    _exit(ok ? 0 : 1);
}

int main() {
    xTaskCreate(run_task, "run", 4096, nullptr, 2, nullptr);
    vTaskStartScheduler();
    for (;;) {}
}

extern "C" void vApplicationMallocFailedHook(void) { printf("FAIL[malloc]\n"); _exit(1); }
extern "C" void vApplicationStackOverflowHook(TaskHandle_t, char*) { printf("FAIL[stack]\n"); _exit(1); }
