/*
 * Compiled and run from tests/smoke.rs. Exercises every C handle type
 * except live-socket connects (those need a Listener pair which is
 * easier to set up from Rust — see live_pair.rs in Task 12).
 */

#include "srtc.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>

int main(void) {
    /* Version macros are visible. */
    printf("srtc version: %d.%d.%d\n",
           SRTC_VERSION_MAJOR, SRTC_VERSION_MINOR, SRTC_VERSION_PATCH);

    /* Muxer roundtrip. */
    srtc_mux_config_t* cfg = srtc_mux_config_new();
    if (!cfg) { fprintf(stderr, "mux_config_new failed\n"); return 1; }
    if (srtc_mux_config_add_video(cfg, 0x1011, SRTC_VIDEO_CODEC_H264) != 0) return 2;
    if (srtc_mux_config_add_klv(cfg, 0x1031, SRTC_KLV_STREAM_TYPE_PRIVATE_DATA, false) != 0) return 3;

    srtc_muxer_t* mux = srtc_muxer_open(cfg);
    if (!mux) { fprintf(stderr, "muxer_open failed: %s\n", srtc_get_last_error_str()); return 4; }

    uint8_t nal[] = { 0, 0, 0, 1, 0x65, 0xAA, 0xAA, 0xAA, 0xAA };
    if (srtc_muxer_push_video(mux, nal, sizeof(nal), 0, true) != 0) {
        fprintf(stderr, "push_video failed: %s\n", srtc_get_last_error_str());
        return 5;
    }
    uint8_t out[4096];
    size_t n = srtc_muxer_pull(mux, out, sizeof(out));
    if (n == 0 || out[0] != 0x47) {
        fprintf(stderr, "pull returned %zu bytes, first=0x%02x\n", n, out[0]);
        return 6;
    }
    srtc_muxer_close(mux);
    srtc_mux_config_free(cfg);

    /* All NULL closes are no-ops. */
    srtc_muxer_close(NULL);
    srtc_mux_sender_close(NULL);
    srtc_managed_mux_sender_close(NULL);
    srtc_ts_sender_close(NULL);
    srtc_managed_ts_sender_close(NULL);
    srtc_raw_sender_close(NULL);
    srtc_managed_raw_sender_close(NULL);

    /* Configs free cleanly. */
    srtc_ts_sender_config_free(srtc_ts_sender_config_new());
    srtc_raw_sender_config_free(srtc_raw_sender_config_new());
    srtc_reconnect_policy_free(srtc_reconnect_policy_new());

    /* Open of a sender against an invalid URL yields NULL with last-error set. */
    srtc_mux_config_t* mc = srtc_mux_config_new();
    srtc_mux_config_add_video(mc, 0x1011, SRTC_VIDEO_CODEC_H264);
    srtc_mux_config_add_klv(mc, 0x1031, SRTC_KLV_STREAM_TYPE_PRIVATE_DATA, false);
    srtc_mux_sender_t* s = srtc_mux_sender_open("not-a-url", mc);
    if (s != NULL) { fprintf(stderr, "expected NULL on invalid url\n"); return 7; }
    int code = srtc_get_last_error();
    if (code == 0) { fprintf(stderr, "expected non-zero last-error\n"); return 8; }
    printf("invalid-url last-error: %d (%s)\n", code, srtc_get_last_error_str());
    srtc_mux_config_free(mc);

    printf("smoke OK\n");
    return 0;
}
