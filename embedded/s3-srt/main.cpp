// S3 — Phase A: SRT caller + listener (FILE mode) on one device over the lossy
// loopback netif. The 564-byte golden is streamed REPEAT times and the listener
// reconstructs it byte-exact. This step runs with the drop filter DISABLED to
// prove the SRT data-plane wiring before adding loss (Task 3 enables it).
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
#include "lossy_netif.h"
}
#include "golden.h"                  // GOLDEN[], GOLDEN_LEN = 564
#include "srt_opts.h"

#define PORT   9001
#define REPEAT 64
static const int STREAM_LEN = (int)GOLDEN_LEN * REPEAT;   // 36096 bytes

static volatile int g_up = 0;
static volatile int g_listen_ready = 0;
static volatile int g_fail = 0;
static const char*  g_where = "";
static uint8_t      g_rxbuf[STREAM_LEN];
static int          g_rxlen = 0;
static SRT_TRACEBSTATS g_rx_stats;

static void tcpip_ready(void*) { g_up = 1; }

static void fail(const char* w) { g_fail = 1; g_where = w; }

// Listener: bind/listen/accept, then srt_recv until STREAM_LEN bytes.
static void* listener_thread(void*) {
    struct sockaddr_in sa; memset(&sa, 0, sizeof sa);
    sa.sin_family = AF_INET; sa.sin_port = lwip_htons(PORT);
    sa.sin_addr.s_addr = lwip_htonl(0x0A000001);   /* 10.0.0.1 (our lossy netif) */
    SRTSOCKET ls = srt_create_socket();
    if (ls == SRT_INVALID_SOCK || s3_apply_opts(ls) != 0) { fail("listen_opts"); return nullptr; }
    if (srt_bind(ls, (sockaddr*)&sa, sizeof sa) == SRT_ERROR) { fail("bind"); return nullptr; }
    if (srt_listen(ls, 1) == SRT_ERROR) { fail("listen"); return nullptr; }
    g_listen_ready = 1;
    SRTSOCKET cs = srt_accept(ls, nullptr, nullptr);
    if (cs == SRT_INVALID_SOCK) { fail("accept"); return nullptr; }
    int got = 0;
    while (got < STREAM_LEN) {
        int n = srt_recv(cs, (char*)g_rxbuf + got, STREAM_LEN - got);
        if (n <= 0) { fail("recv"); break; }
        got += n;
    }
    g_rxlen = got;
    // Snapshot receiver stats AFTER the full stream is in: pktRcvRetransTotal is
    // now final and counts every retransmitted packet we had to receive to
    // recover the ~20% the lossy netif dropped. (The sender's pktRetransTotal,
    // sampled right after its send loop, would still be 0 in FILE mode — that
    // loop only buffers; retransmission happens during srt_close's linger.)
    srt_bstats(cs, &g_rx_stats, 0);
    srt_close(cs);
    srt_close(ls);
    return nullptr;
}

// Caller: connect, then srt_send the golden REPEAT times.
static void* caller_thread(void*) {
    while (!g_listen_ready) vTaskDelay(pdMS_TO_TICKS(1));
    struct sockaddr_in sa; memset(&sa, 0, sizeof sa);
    sa.sin_family = AF_INET; sa.sin_port = lwip_htons(PORT);
    sa.sin_addr.s_addr = lwip_htonl(0x0A000001);   /* 10.0.0.1 (our lossy netif) */
    SRTSOCKET cs = srt_create_socket();
    if (cs == SRT_INVALID_SOCK || s3_apply_opts(cs) != 0) { fail("call_opts"); return nullptr; }
    if (srt_connect(cs, (sockaddr*)&sa, sizeof sa) == SRT_ERROR) { fail("connect"); return nullptr; }
    // Handshake (+ KM in Phase B) is complete now; loss may be enabled.
    lossy_set_enabled(S3_LOSS_ENABLED);
    for (int r = 0; r < REPEAT && !g_fail; r++) {
        int off = 0;
        while (off < (int)GOLDEN_LEN) {
            int n = srt_send(cs, (const char*)GOLDEN + off, (int)GOLDEN_LEN - off);
            if (n <= 0) { fail("send"); break; }
            off += n;
        }
    }
    srt_close(cs);
    return nullptr;
}

static void run_task(void*) {
    tcpip_init(tcpip_ready, nullptr);
    while (!g_up) vTaskDelay(pdMS_TO_TICKS(1));
    lossy_netif_up();
    lossy_set_enabled(0);              // off until the caller connects

    if (srt_startup() < 0) { fail("startup"); }

    pthread_t lt, ct;
    pthread_attr_t attr; pthread_attr_init(&attr);
    pthread_attr_setstacksize(&attr, 32768);         // libsrt API call chains are deep
    pthread_create(&lt, &attr, listener_thread, nullptr);
    pthread_create(&ct, &attr, caller_thread, nullptr);
    pthread_join(ct, nullptr);
    pthread_join(lt, nullptr);

    // Verify: full length + every 564-byte chunk equals the golden.
    int bytes_ok = (g_rxlen == STREAM_LEN);
    for (int r = 0; bytes_ok && r < REPEAT; r++)
        if (memcmp(g_rxbuf + r * GOLDEN_LEN, GOLDEN, GOLDEN_LEN) != 0) bytes_ok = 0;

    // dropped = packets the netif deterministically injected as loss (0 when the
    // filter is off). In FILE mode the ONLY way the stream still reconstructs
    // byte-exact (bytes_ok) is SRT retransmitting each dropped packet, so
    // dropped>0 && bytes_ok proves ARQ recovery. rcv_loss is SRT's own
    // receiver-side loss tally, reported for corroboration (it can read 1 even
    // with no injected loss, so it is NOT the gate).
    unsigned dropped = lossy_dropped_count();
    int rcvloss = g_rx_stats.pktRcvLossTotal;

#if S3_LOSS_ENABLED
    int ok = !g_fail && bytes_ok && dropped > 0;     // injected loss recovered byte-exact
    if (ok) printf("PASS: s3_srt_plain (GOLDEN x %d recovered byte-exact under ~20%% loss, dropped=%u, rcv_loss=%d)\n",
                   REPEAT, dropped, rcvloss);
    else    printf("FAIL[s3_srt_plain]: where=%s rxlen=%d/%d bytes_ok=%d dropped=%u rcv_loss=%d\n",
                   g_where, g_rxlen, STREAM_LEN, bytes_ok, dropped, rcvloss);
#else
    int ok = !g_fail && bytes_ok;                    // clean delivery (no loss yet)
    if (ok) printf("PASS: s3_srt_clean (GOLDEN x %d delivered, no loss)\n", REPEAT);
    else    printf("FAIL[s3_srt_clean]: where=%s rxlen=%d/%d bytes_ok=%d\n",
                   g_where, g_rxlen, STREAM_LEN, bytes_ok);
#endif
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
