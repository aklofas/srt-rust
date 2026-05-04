/*
 * send_synthetic.c — minimal C consumer of libsrtc.
 *
 * Build:
 *   cc -I target/debug/include -L target/debug \
 *      -Wall -Werror -o /tmp/send_synthetic \
 *      crates/srt-c/examples/c/send_synthetic.c -lsrtc
 *
 * Run:
 *   LD_LIBRARY_PATH=target/debug ./send_synthetic 127.0.0.1:9000
 *
 * Receiver side (separate terminal):
 *   srt-live-transmit srt://:9000 file:///tmp/out.ts
 *
 * Mirrors crates/srt-core/examples/pipeline_send_to_socket.rs.
 */

#include "srtc.h"
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

static void make_nal(uint8_t *buf, size_t len) {
    buf[0] = 0; buf[1] = 0; buf[2] = 0; buf[3] = 1; buf[4] = 0x65;
    memset(buf + 5, 0xAA, len - 5);
}

/* 16-byte UAS Datalink LS UL + BER short-form length 16 + 16 payload bytes. */
static size_t make_klv(uint8_t *buf, uint8_t seq) {
    static const uint8_t ul[16] = {
        0x06, 0x0E, 0x2B, 0x34, 0x02, 0x0B, 0x01, 0x01,
        0x0E, 0x01, 0x03, 0x01, 0x01, 0x00, 0x00, 0x00,
    };
    memcpy(buf, ul, 16);
    buf[16] = 16;
    memset(buf + 17, seq, 16);
    return 33;
}

int main(int argc, char **argv) {
    const char *host_port = (argc > 1) ? argv[1] : "127.0.0.1:9000";
    char url[256];
    snprintf(url, sizeof(url), "srt://%s", host_port);
    fprintf(stderr, "sending to %s\n", url);

    srtc_mux_config_t *cfg = srtc_mux_config_new();
    srtc_program_handle_t prog = srtc_mux_config_add_program(cfg, 1, 0x1000);
    srtc_mux_config_add_video_stream(cfg, prog, 0x1011, SRTC_VIDEO_CODEC_H264);
    srtc_mux_config_add_klv_stream(cfg, prog, 0x1031, SRTC_KLV_STREAM_TYPE_PRIVATE_DATA, false);

    srtc_mux_sender_t *s = srtc_mux_sender_open(url, cfg);
    if (!s) {
        fprintf(stderr, "open failed: %s\n", srtc_get_last_error_str());
        srtc_mux_config_free(cfg);
        return 1;
    }
    srtc_mux_config_free(cfg);

    uint8_t nal[500];
    uint8_t klv[64];
    for (int i = 0; i < 5; i++) {
        make_nal(nal, sizeof(nal));
        size_t klv_len = make_klv(klv, (uint8_t)i);

        int rc = srtc_mux_sender_send_video(s, nal, sizeof(nal), i * 33000, i == 0);
        if (rc != 0) {
            fprintf(stderr, "send_video[%d] failed: %s\n", i, srtc_get_last_error_str());
            srtc_mux_sender_close(s);
            return 2;
        }

        rc = srtc_mux_sender_send_klv(s, klv, klv_len, i * 33000);
        if (rc != 0) {
            fprintf(stderr, "send_klv[%d] failed: %s\n", i, srtc_get_last_error_str());
            srtc_mux_sender_close(s);
            return 3;
        }

        usleep(33 * 1000);
    }

    fprintf(stderr, "done. closing.\n");
    srtc_mux_sender_close(s);
    usleep(200 * 1000);
    return 0;
}
