// srt-recv — SRT INGRESS: the reverse of `example`. A bare-metal SRT
// LISTENER on a real lan9118 NIC accepts an inbound connection from a HOST
// tst-srt CALLER (example/host's `--send` mode) over QEMU SLIRP user-net,
// receives the 564-byte golden video-roundtrip stream, and demuxes it
// ON-DEVICE via the offline `tst_demuxer_*` C ABI (the same offline surface
// baremetal-qemu-c/firmware/main.c exercises, here linked into a FreeRTOS +
// lwIP binary for the first time). Unlike `example` (where the firmware only
// proves it SENT and the host is the byte-exact verifier), here the FIRMWARE
// is the authoritative verifier: it checks both the demuxed event census
// AND the raw video payload bytes against the golden — the roles are
// mirrored, not just the data direction.
//
// Topology: `-nic user,model=lan9118,hostfwd=udp::H-:9003` (SLIRP UDP port
// forward) — SRT's base protocol is UDP, so the inbound handshake traverses
// SLIRP's NAT the same way an outbound one does in `example`. Verified by a
// standalone probe firmware before this file was written: a minimal SRT
// listener bound to the guest NIC accepted a connection from the host
// launched with the same hostfwd flag on the first attempt — no fallback
// topology (firmware-as-caller) was needed.
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
#include "tstrans.h"                 // tst_demuxer_* offline C ABI (libtstrans_firmware.a)

#define PORT 9003

static volatile int g_up = 0;
static volatile int g_fail = 0;
static const char*  g_where = "";
static uint8_t      g_rxbuf[GOLDEN_LEN];
static int          g_rxlen = 0;

static void tcpip_ready(void*) { g_up = 1; }

// First failure wins, and the label prints IMMEDIATELY (matches loopback-arq's
// fail() rationale): if something later still wedges, the transcript already
// names where things went wrong instead of an anonymous timeout.
static void fail(const char* w) {
    if (g_fail) return;
    g_fail = 1; g_where = w;
    printf("FAIL-DETAIL: where=%s\n", w); fflush(stdout);
}

// Dedicated FreeRTOS task: poll the lan9118 RX FIFO into lwIP (mirrors example).
static void rx_poll_task(void*) {
    for (;;) { lan9118_poll(); vTaskDelay(1); }
}

// Mirror of tst_integration::scenarios::synthetic_h264_idr() — the raw H.264
// NAL the golden's video PES payload was muxed from (Annex-B start code +
// IDR NAL header 0x65 + 15 bytes 0xA5^i). Same fixture baremetal-qemu-c's
// firmware/main.c's make_input() reconstructs; both goldens derive from the
// identical committed video-roundtrip/output.ts, so the expected shape here
// is byte-for-byte the same.
static void expected_nal_input(uint8_t buf[20]) {
    buf[0] = 0x00; buf[1] = 0x00; buf[2] = 0x00; buf[3] = 0x01;
    buf[4] = 0x65;
    for (int i = 0; i < 15; i++) buf[5 + i] = (uint8_t)(0xA5 ^ i);
}

// Demux the received TS on-device and assert BOTH the event census and the
// raw video payload bytes against the golden's known shape (one ProgramMap:
// program 1, PMT PID 0x1000, PCR PID 0x1011, one H.264 video stream; one
// video Sample: PTS 0, no DTS, the 20-byte NAL byte-exact). Returns 1 on
// success, 0 on failure (fail() has already recorded where=).
static int demux_and_verify(const uint8_t* ts, size_t ts_len) {
    uint8_t expected[20];
    expected_nal_input(expected);

    struct TstDemuxer* dmx = tst_demuxer_open();
    if (!dmx) { fail("demuxer_open"); return 0; }
    // feed+flush in one shot: the recv loop below already accumulated the
    // full GOLDEN_LEN before calling here, so there is nothing incremental
    // left to prove by feeding in smaller pieces (the recv loop itself is
    // where the "live" streaming happens, one SRT message at a time).
    if (tst_demuxer_feed(dmx, ts, ts_len) != 0)  { fail("demuxer_feed");  tst_demuxer_close(dmx); return 0; }
    if (tst_demuxer_flush(dmx) != 0)             { fail("demuxer_flush"); tst_demuxer_close(dmx); return 0; }

    int saw_program_map = 0, saw_video_sample = 0, events = 0;
    for (;;) {
        tst_event_t ev;
        int rc = tst_demuxer_next_event(dmx, &ev);
        if (rc == TST_E_NOT_AVAILABLE) break;                 /* drained */
        if (rc != 0) { fail("next_event"); tst_demuxer_close(dmx); return 0; }
        if (++events > 64) { fail("event_flood"); tst_demuxer_close(dmx); return 0; }

        if (ev.kind == TST_EVENT_KIND_PROGRAM_MAP) {
            if (ev.u.program_map.program_number != 1)  { fail("pm.program_number"); tst_demuxer_close(dmx); return 0; }
            if (ev.u.program_map.pmt_pid != 0x1000)     { fail("pm.pmt_pid");        tst_demuxer_close(dmx); return 0; }
            if (ev.u.program_map.pcr_pid != 0x1011)     { fail("pm.pcr_pid");        tst_demuxer_close(dmx); return 0; }
            if (ev.u.program_map.stream_count != 1 || !ev.u.program_map.streams) {
                fail("pm.stream_count"); tst_demuxer_close(dmx); return 0;
            }
            const tst_stream_info_t* si = &ev.u.program_map.streams[0];
            if (si->pid != 0x1011)                       { fail("si.pid");          tst_demuxer_close(dmx); return 0; }
            if (si->stream_kind != TST_STREAM_KIND_VIDEO) { fail("si.stream_kind");  tst_demuxer_close(dmx); return 0; }
            if (si->codec != TST_VIDEO_CODEC_H264)        { fail("si.codec");        tst_demuxer_close(dmx); return 0; }
            saw_program_map = 1;
        } else if (ev.kind == TST_EVENT_KIND_SAMPLE && ev.u.sample.stream_kind == TST_STREAM_KIND_VIDEO) {
            if (ev.u.sample.pid != 0x1011)                { fail("s.pid");           tst_demuxer_close(dmx); return 0; }
            if (ev.u.sample.codec != TST_VIDEO_CODEC_H264) { fail("s.codec");        tst_demuxer_close(dmx); return 0; }
            if (ev.u.sample.pts != 0)                     { fail("s.pts");           tst_demuxer_close(dmx); return 0; }
            if (ev.u.sample.dts != INT64_MIN)             { fail("s.dts");           tst_demuxer_close(dmx); return 0; } /* absent-DTS sentinel */
            if (ev.u.sample.payload_len != 20 || !ev.u.sample.payload) {
                fail("s.payload_len"); tst_demuxer_close(dmx); return 0;
            }
            if (memcmp(ev.u.sample.payload, expected, 20) != 0) { fail("s.payload_bytes"); tst_demuxer_close(dmx); return 0; }
            if (ev.u.sample.nal_count != 1 || !ev.u.sample.nals) {
                fail("s.nal_count"); tst_demuxer_close(dmx); return 0;
            }
            const tst_nal_t* nal = &ev.u.sample.nals[0];
            if (nal->nal_type != 5)              { fail("nal.nal_type");    tst_demuxer_close(dmx); return 0; }
            if (nal->ref_idc_or_layer_id != 3)   { fail("nal.ref_idc");     tst_demuxer_close(dmx); return 0; }
            /* H.264 NAL views strip the start code AND the 1-byte header. */
            if (nal->payload_len != 15 || !nal->payload) { fail("nal.payload_len"); tst_demuxer_close(dmx); return 0; }
            if (nal->payload[0] != 0xA5)          { fail("nal.payload0");   tst_demuxer_close(dmx); return 0; }
            saw_video_sample = 1;
        }
    }
    tst_demuxer_close(dmx);
    if (!saw_program_map)  { fail("missing_program_map"); return 0; }
    if (!saw_video_sample) { fail("missing_video_sample"); return 0; }
    return 1;
}

// Listener: bind ANY-local:PORT, listen, accept once, then srt_recv until
// the golden's compile-time-known GOLDEN_LEN bytes have arrived. LIVE mode
// (srt_opts.h default) matches the host tst-srt CALLER — a Socket built via
// SocketBuilder, also LIVE by default (see example/host/src/main.rs's
// ListenerBuilder counterpart for the "why LIVE" rationale).
static void* listener_thread(void*) {
    struct sockaddr_in sa; memset(&sa, 0, sizeof sa);
    sa.sin_family = AF_INET; sa.sin_port = lwip_htons(PORT);
    sa.sin_addr.s_addr = lwip_htonl(0x00000000);   /* 0.0.0.0 - any local */
    SRTSOCKET ls = srt_create_socket();
    if (ls == SRT_INVALID_SOCK)                                { fail("listen_create"); return nullptr; }
    if (srt_apply_opts(ls) != 0)                                { fail("listen_opts");   srt_close(ls); return nullptr; }
    if (srt_bind(ls, (sockaddr*)&sa, sizeof sa) == SRT_ERROR)   { fail("bind");           srt_close(ls); return nullptr; }
    if (srt_listen(ls, 1) == SRT_ERROR)                         { fail("listen");         srt_close(ls); return nullptr; }
    // Signals the gate script (polling QEMU's captured output) that bind+
    // listen succeeded and it is safe to launch the host --send caller —
    // the mirror of example/host printing "host-ready" before its firmware
    // caller is launched.
    printf("guest-ready\n"); fflush(stdout);
    SRTSOCKET cs = srt_accept(ls, nullptr, nullptr);
    if (cs == SRT_INVALID_SOCK)                                 { fail("accept");         srt_close(ls); return nullptr; }

    // LIVE/message mode: srt_recv requires the buffer to be at least the
    // negotiated payload size (~1456 B), even though the golden message is
    // only 564 B — passing a smaller tail slice throws "Incorrect use of
    // Message API" (same constraint example/host/src/main.rs documents on
    // the mirror side of this wire shape). Recv into a fixed full-size temp
    // buffer each call, then copy the (possibly truncated, at GOLDEN_LEN)
    // tail into place.
    uint8_t tmp[2048];
    int got = 0;
    while (got < (int)GOLDEN_LEN) {
        int n = srt_recv(cs, (char*)tmp, sizeof tmp);
        if (n <= 0) { fail("recv"); break; }
        int take = n;
        if (got + take > (int)GOLDEN_LEN) take = (int)GOLDEN_LEN - got;
        memcpy(g_rxbuf + got, tmp, (size_t)take);
        got += take;
    }
    g_rxlen = got;
    srt_close(cs);
    srt_close(ls);
    return nullptr;
}

static void run_task(void*) {
    tcpip_init(tcpip_ready, nullptr);
    while (!g_up) vTaskDelay(pdMS_TO_TICKS(1));
    lan9118_netif_up(10,0,2,15, 10,0,2,2);             // guest 10.0.2.15, gw 10.0.2.2
    // rx poll task at priority 3: below tcpip (4) so pushed frames are processed
    // promptly, above run_task (2). NOTE configMAX_PRIORITIES=5 -> max prio is 4.
    xTaskCreate(rx_poll_task, "rx", 1024, nullptr, 3, nullptr);

    // srt_startup failure is terminal: every later SRT call would run in an
    // invalid init state. Fail fast before the listener thread exists.
    if (srt_startup() < 0) { printf("FAIL[srt_recv]: where=startup\n"); fflush(stdout); _exit(1); }

    pthread_t lt;
    pthread_attr_t attr; pthread_attr_init(&attr);
    pthread_attr_setstacksize(&attr, 32768);         // libsrt API call chains are deep
    if (pthread_create(&lt, &attr, listener_thread, nullptr) != 0) {
        printf("FAIL[srt_recv]: pthread_create(listener)\n"); fflush(stdout); _exit(1);
    }
    pthread_join(lt, nullptr);

    int recv_ok = !g_fail && g_rxlen == (int)GOLDEN_LEN;
    int demux_ok = recv_ok && demux_and_verify(g_rxbuf, (size_t)g_rxlen);
    int ok = recv_ok && demux_ok;

    if (ok) {
        printf("PASS: srt_recv (%d bytes received over SRT, demuxed on-device: "
               "census + video payload byte-exact)\n", g_rxlen);
    } else if (!recv_ok) {
        printf("FAIL[srt_recv]: where=%s rxlen=%d/%u\n", g_where, g_rxlen, (unsigned)GOLDEN_LEN);
    } else {
        printf("FAIL[srt_recv]: where=%s (on-device demux verification)\n", g_where);
    }
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
