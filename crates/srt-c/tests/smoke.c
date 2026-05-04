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

    /* Muxer roundtrip — single program. */
    srtc_mux_config_t* cfg = srtc_mux_config_new();
    if (!cfg) { fprintf(stderr, "mux_config_new failed\n"); return 1; }
    srtc_program_handle_t p1 = srtc_mux_config_add_program(cfg, 1, 0x1000);
    if (p1 == SRTC_INVALID_PROGRAM_HANDLE) {
        fprintf(stderr, "add_program failed\n"); return 2;
    }
    srtc_video_stream_handle_t hv = srtc_mux_config_add_video_stream(
        cfg, p1, 0x1011, SRTC_VIDEO_CODEC_H264);
    if (hv == SRTC_INVALID_STREAM_HANDLE) { fprintf(stderr, "add_video_stream failed\n"); return 3; }
    srtc_klv_stream_handle_t hk = srtc_mux_config_add_klv_stream(
        cfg, p1, 0x1031, SRTC_KLV_STREAM_TYPE_PRIVATE_DATA, false);
    if (hk == SRTC_INVALID_STREAM_HANDLE) { fprintf(stderr, "add_klv_stream failed\n"); return 4; }

    srtc_muxer_t* mux = srtc_muxer_open(cfg);
    if (!mux) { fprintf(stderr, "muxer_open failed: %s\n", srtc_get_last_error_str()); return 5; }

    uint8_t nal[] = { 0, 0, 0, 1, 0x65, 0xAA, 0xAA, 0xAA, 0xAA };
    if (srtc_muxer_push_video_to(mux, hv, nal, sizeof(nal), 0, true) != 0) {
        fprintf(stderr, "push_video_to failed: %s\n", srtc_get_last_error_str());
        return 6;
    }
    uint8_t out[4096];
    size_t n = srtc_muxer_pull(mux, out, sizeof(out));
    if (n == 0 || out[0] != 0x47) {
        fprintf(stderr, "pull returned %zu bytes, first=0x%02x\n", n, out[0]);
        return 7;
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
    srtc_program_handle_t mc_prog = srtc_mux_config_add_program(mc, 1, 0x1000);
    srtc_video_stream_handle_t mc_hv = srtc_mux_config_add_video_stream(
        mc, mc_prog, 0x1011, SRTC_VIDEO_CODEC_H264);
    srtc_klv_stream_handle_t mc_hk = srtc_mux_config_add_klv_stream(
        mc, mc_prog, 0x1031, SRTC_KLV_STREAM_TYPE_PRIVATE_DATA, false);
    (void)mc_hv; (void)mc_hk;
    srtc_mux_sender_t* s = srtc_mux_sender_open("not-a-url", mc);
    if (s != NULL) { fprintf(stderr, "expected NULL on invalid url\n"); return 8; }
    int code = srtc_get_last_error();
    if (code == 0) { fprintf(stderr, "expected non-zero last-error\n"); return 9; }
    printf("invalid-url last-error: %d (%s)\n", code, srtc_get_last_error_str());
    srtc_mux_config_free(mc);

    /* Multi-stream muxer: two video PIDs + one KLV PID, each with a handle. */
    srtc_mux_config_t* mcfg = srtc_mux_config_new();
    if (!mcfg) return 10;
    srtc_program_handle_t mp = srtc_mux_config_add_program(mcfg, 1, 0x1000);
    srtc_video_stream_handle_t h_eo =
        srtc_mux_config_add_video_stream(mcfg, mp, 0x1011, SRTC_VIDEO_CODEC_H264);
    srtc_video_stream_handle_t h_ir =
        srtc_mux_config_add_video_stream(mcfg, mp, 0x1012, SRTC_VIDEO_CODEC_H264);
    srtc_klv_stream_handle_t h_klv =
        srtc_mux_config_add_klv_stream(mcfg, mp, 0x1031, SRTC_KLV_STREAM_TYPE_PRIVATE_DATA, false);
    if (h_eo == SRTC_INVALID_STREAM_HANDLE) return 11;
    if (h_ir == SRTC_INVALID_STREAM_HANDLE) return 12;
    if (h_klv == SRTC_INVALID_STREAM_HANDLE) return 13;
    /* packed handle: VideoStreamHandle::pack(0,0)=0, pack(0,1)=1 */
    if (h_eo != 0 || h_ir != 1) {
        fprintf(stderr, "video handles not 0,1: got %u,%u\n", h_eo, h_ir);
        return 14;
    }
    if (h_klv != 0) { fprintf(stderr, "klv handle not 0: got %u\n", h_klv); return 15; }

    srtc_muxer_t* mmux = srtc_muxer_open(mcfg);
    srtc_mux_config_free(mcfg);
    if (!mmux) return 16;

    if (srtc_muxer_push_video_to(mmux, h_eo, nal, sizeof(nal), 0, true) != 0) return 17;
    if (srtc_muxer_push_video_to(mmux, h_ir, nal, sizeof(nal), 1000, true) != 0) return 18;
    uint8_t klv[] = {
        0x06, 0x0e, 0x2b, 0x34, 0x02, 0x0b, 0x01, 0x01,
        0x0e, 0x01, 0x03, 0x01, 0x01, 0x00, 0x00, 0x00,
    };
    if (srtc_muxer_push_klv_to(mmux, h_klv, klv, sizeof(klv), 2000) != 0) return 19;

    /* Drain and confirm aligned TS bytes. */
    size_t mn = srtc_muxer_pull(mmux, out, sizeof(out));
    if (mn == 0 || (mn % 188) != 0 || out[0] != 0x47) {
        fprintf(stderr, "multi-stream pull: n=%zu first=0x%02x\n", mn, out[0]);
        return 20;
    }

    /* Out-of-range handle is rejected with SRTC_E_INVALID_USAGE. */
    int rc = srtc_muxer_push_video_to(mmux, 99, nal, sizeof(nal), 3000, true);
    if (rc != SRTC_E_INVALID_USAGE) {
        fprintf(stderr, "expected SRTC_E_INVALID_USAGE, got %d\n", rc);
        return 21;
    }

    srtc_muxer_close(mmux);

    /* Multi-program config: verify add_program returns sequential ordinals,
     * stream-add routes to the right program, build succeeds. */
    {
        srtc_mux_config_t* mpcfg = srtc_mux_config_new();
        if (!mpcfg) return 22;

        srtc_program_handle_t pp1 = srtc_mux_config_add_program(mpcfg, 1, 0x1000);
        srtc_program_handle_t pp2 = srtc_mux_config_add_program(mpcfg, 2, 0x1100);
        if (pp1 != 0) {
            fprintf(stderr, "program 1 handle not 0: got %u\n", pp1);
            return 23;
        }
        if (pp2 != 1) {
            fprintf(stderr, "program 2 handle not 1: got %u\n", pp2);
            return 24;
        }

        /* VideoStreamHandle::pack(0,0)=0 for program 1 stream 0 */
        srtc_video_stream_handle_t vp1 = srtc_mux_config_add_video_stream(
            mpcfg, pp1, 0x1011, SRTC_VIDEO_CODEC_H264);
        /* VideoStreamHandle::pack(1,0)=0x10 for program 2 stream 0 */
        srtc_video_stream_handle_t vp2 = srtc_mux_config_add_video_stream(
            mpcfg, pp2, 0x1111, SRTC_VIDEO_CODEC_H265);
        if (vp1 == SRTC_INVALID_STREAM_HANDLE || vp2 == SRTC_INVALID_STREAM_HANDLE) {
            fprintf(stderr, "video stream add failed\n"); return 25;
        }
        /* Handles from different programs must be distinct. */
        if (vp1 == vp2) {
            fprintf(stderr, "expected distinct handles; got %u == %u\n", vp1, vp2);
            return 26;
        }

        srtc_mux_config_add_klv_stream(mpcfg, pp1, 0x1031, SRTC_KLV_STREAM_TYPE_PRIVATE_DATA, false);
        srtc_mux_config_add_klv_stream(mpcfg, pp2, 0x1131, SRTC_KLV_STREAM_TYPE_PRIVATE_DATA, false);

        srtc_muxer_t* mpmux = srtc_muxer_open(mpcfg);
        if (!mpmux) {
            fprintf(stderr, "multi-program muxer_open failed: %s\n", srtc_get_last_error_str());
            srtc_mux_config_free(mpcfg);
            return 27;
        }

        srtc_mux_config_free(mpcfg);
        srtc_muxer_close(mpmux);
        fprintf(stderr, "multi-program config OK\n");
    }

    /* Stats accessor smoke. Confirms srtc_muxer_get_stats / reset_stats
     * link, return 0 on success, populate per_stream_count from the
     * eager-on-construction config, and reject null pointers with a
     * non-zero return. Live round-trip is covered by the Rust
     * integration test crates/srt-c/tests/stats.rs. */
    {
        srtc_mux_config_t* scfg = srtc_mux_config_new();
        srtc_program_handle_t sp = srtc_mux_config_add_program(scfg, 1, 0x1000);
        srtc_mux_config_add_video_stream(scfg, sp, 0x0100, SRTC_VIDEO_CODEC_H264);
        srtc_mux_config_add_klv_stream(scfg, sp, 0x0101, SRTC_KLV_STREAM_TYPE_PRIVATE_DATA, false);
        srtc_muxer_t* sm = srtc_muxer_open(scfg);
        if (!sm) { fprintf(stderr, "stats: mux open failed: %s\n", srtc_get_last_error_str()); return 28; }

        srtc_muxer_stats_t st;
        int src = srtc_muxer_get_stats(sm, &st);
        if (src != 0) { fprintf(stderr, "stats: get_stats failed: %d\n", src); return 29; }
        /* One video + one KLV = 2 streams; per_stream_count mirrors the
         * stream count declared in the config at construction time. */
        if (st.per_stream_count != 2) {
            fprintf(stderr, "stats: expected 2 streams, got %u\n", st.per_stream_count);
            return 30;
        }

        src = srtc_muxer_reset_stats(sm);
        if (src != 0) { fprintf(stderr, "stats: reset_stats failed: %d\n", src); return 31; }

        /* Null-pointer paths must return non-zero — the C API treats null
         * handles and null out-pointers as caller bugs and reports them
         * immediately rather than crashing. */
        if (srtc_muxer_get_stats(NULL, &st) == 0) {
            fprintf(stderr, "stats: null muxer should fail\n");
            return 32;
        }
        if (srtc_muxer_get_stats(sm, NULL) == 0) {
            fprintf(stderr, "stats: null out should fail\n");
            return 33;
        }

        srtc_muxer_close(sm);
        srtc_mux_config_free(scfg);
        fprintf(stderr, "stats smoke OK\n");
    }

    printf("smoke OK\n");
    return 0;
}
