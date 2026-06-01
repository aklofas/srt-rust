#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <string.h>
#include "tstrans.h"
#include "golden.h"   /* generated: GOLDEN[] + GOLDEN_LEN (from output.ts) */

/* Mirror of tst_integration::scenarios::synthetic_h264_idr():
   Annex-B start code + IDR NAL header (0x65) + 15 bytes 0xA5 ^ i. */
static void make_input(uint8_t buf[20]) {
    buf[0] = 0x00; buf[1] = 0x00; buf[2] = 0x00; buf[3] = 0x01;
    buf[4] = 0x65;
    for (int i = 0; i < 15; i++) buf[5 + i] = (uint8_t)(0xA5 ^ i);
}

static int fail(const char *what, size_t got) {
    printf("FAIL[c_firmware] %s: produced %u bytes, golden %u\n", what, (unsigned)got, (unsigned)GOLDEN_LEN);
    return 1;
}

int main(void) {
    struct tst_mux_config_t *cfg = tst_mux_config_new();
    if (!cfg) return fail("config_new", 0);
    tst_program_handle_t prog = tst_mux_config_add_program(cfg, 1, 0x1000);
    tst_video_stream_handle_t vid =
        tst_mux_config_add_video_stream(cfg, prog, 0x1011, TST_VIDEO_CODEC_H264);
    (void)vid;

    struct tst_muxer_t *mux = tst_muxer_open(cfg); /* borrows cfg */
    tst_mux_config_free(cfg);
    if (!mux) return fail("muxer_open", 0);

    uint8_t input[20];
    make_input(input);
    if (tst_muxer_push_video(mux, input, sizeof input, /*pts_90khz=*/0, /*key_frame=*/true) != 0)
        return fail("push_video", 0);

    /* Drain in 1316-byte (7x188) chunks, accumulate. */
    static uint8_t out[4096];
    size_t total = 0;
    uint8_t chunk[1316];
    for (;;) {
        size_t n = tst_muxer_pull(mux, chunk, sizeof chunk);
        if (n == 0) break;
        if (total + n > sizeof out) return fail("overflow", total + n);
        memcpy(out + total, chunk, n);
        total += n;
    }
    tst_muxer_close(mux);

    if (total != (size_t)GOLDEN_LEN || memcmp(out, GOLDEN, total) != 0)
        return fail("mismatch", total);

    printf("PASS: c_firmware (%u bytes)\n", (unsigned)total);
    return 0;
}
