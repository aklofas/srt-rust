/*
 * hello_world.c — the smallest tst-c example.
 *
 * Build (Linux x86_64, with libtstrans built locally):
 *   cd crates/tst-c
 *   cargo build
 *   cc -I include -L target/debug \
 *      -Wall -Werror -o /tmp/hello_world \
 *      examples/c/getting-started/hello_world.c -ltstrans
 *
 * Run:
 *   LD_LIBRARY_PATH=target/debug /tmp/hello_world
 *
 * What it does: builds 1 MPEG-TS frame containing 1 H.264 access unit
 * and 1 ST 0601-shaped KLV record using the muxer-only handle, drains
 * the resulting TS packets, prints a byte count. No SRT, no files.
 *
 * From here:
 *   - For SRT sending:        muxing/send_synthetic.c
 *   - For multi-camera mux:   muxing/mux_dual_camera.c
 *   - For multi-program mux:  muxing/mux_two_programs.c
 */

#include "tstrans.h"
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

int main(void) {
    /*
     * ── Step 1: Build the mux config ────────────────────────────────────
     *
     * One program with one H.264 video stream + one async-KLV stream.
     * Async (PrivateData, stream_type 0x06) is the simplest KLV mode —
     * payload passes through verbatim, no AU cell wrap, no per-record PTS
     * (carries_pts = false).
     */
    tst_mux_config_t *cfg = tst_mux_config_new();
    if (!cfg) {
        fprintf(stderr, "tst_mux_config_new failed\n");
        return 1;
    }
    tst_program_handle_t prog = tst_mux_config_add_program(cfg, /*program_number=*/ 1,
                                                           /*pmt_pid=*/ 0x1000);
    tst_mux_config_add_video_stream(cfg, prog, /*pid=*/ 0x100, TST_VIDEO_CODEC_H264);
    tst_mux_config_add_klv_stream(cfg, prog, /*pid=*/ 0x101,
                                  TST_KLV_STREAM_TYPE_PRIVATE_DATA,
                                  /*carries_pts=*/ false);

    /*
     * ── Step 2: Open the muxer ──────────────────────────────────────────
     *
     * The muxer-only handle has no transport — pulled bytes go nowhere
     * unless we drain them ourselves. tst_muxer_open consumes the config
     * by-value internally, so the caller may free `cfg` immediately.
     */
    tst_muxer_t *mux = tst_muxer_open(cfg);
    tst_mux_config_free(cfg);
    if (!mux) {
        fprintf(stderr, "tst_muxer_open failed: %s\n", tst_get_last_error_str());
        return 2;
    }

    /*
     * ── Step 3: Push 1 video AU + 1 KLV record ──────────────────────────
     *
     * The video AU is a single Annex-B-framed AUD NAL (NAL unit type 9 =
     * access unit delimiter). Real callers feed encoder output; we hand-
     * roll one byte string here so the example has no codec dependency.
     */
    static const uint8_t aud_nal[] = { 0x00, 0x00, 0x00, 0x01, 0x09, 0x10 };
    if (tst_muxer_push_video(mux, aud_nal, sizeof(aud_nal),
                             /*pts_90khz=*/ 0, /*key_frame=*/ true) != 0) {
        fprintf(stderr, "push_video failed: %s\n", tst_get_last_error_str());
        tst_muxer_close(mux);
        return 3;
    }

    /*
     * Push a synthetic ST 0601 KLV blob. 16-byte UAS Datalink LS UL +
     * BER short-form length 16 + 16 zero bytes. Async-KLV streams pass
     * the bytes through unchanged (no AU cell header prepended).
     */
    static const uint8_t klv[] = {
        /* 16-byte UAS Datalink LS UL (header) */
        0x06, 0x0E, 0x2B, 0x34, 0x02, 0x0B, 0x01, 0x01,
        0x0E, 0x01, 0x03, 0x01, 0x01, 0x00, 0x00, 0x00,
        /* BER short-form length 16 */
        0x10,
        /* 16 payload bytes (synthetic) */
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    };
    if (tst_muxer_push_klv(mux, klv, sizeof(klv), /*pts_90khz=*/ 0) != 0) {
        fprintf(stderr, "push_klv failed: %s\n", tst_get_last_error_str());
        tst_muxer_close(mux);
        return 4;
    }

    /*
     * ── Step 4: Drain TS packets out of the muxer ───────────────────────
     *
     * tst_muxer_pull writes 188-byte TS packets into a caller-provided
     * buffer; returns 0 when nothing is queued or when the buffer is
     * smaller than the next chunk. Here we pull one packet at a time.
     */
    uint8_t packet[188];
    size_t total_bytes = 0;
    size_t total_packets = 0;
    for (;;) {
        size_t n = tst_muxer_pull(mux, packet, sizeof(packet));
        if (n == 0) break;
        total_bytes += n;
        total_packets += 1;
    }

    tst_muxer_close(mux);

    printf("Built %zu bytes of MPEG-TS (%zu packets) containing 1 video AU + 1 KLV record.\n",
           total_bytes, total_packets);
    printf("Next: muxing/send_synthetic.c sends synthetic frames over SRT.\n");
    return 0;
}
