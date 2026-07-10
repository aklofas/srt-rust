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

/* Demux-phase failure: prints the offending check + observed value.
   (newlib-nano printf lacks %lld — 64-bit observations are truncated to
   their low 32 bits for the transcript; the compare itself is full-width.) */
static int fail_demux(const char *what, unsigned got) {
    printf("FAIL[c_firmware] demux %s (got=%u)\n", what, got);
    return 1;
}

/* Feed the produced TS back through the offline demuxer and read the typed
   event structs. On this 32-bit target that exercises the pointer-width-
   sensitive layouts (tst_event_t, tst_stream_info_t, tst_nal_t) across the
   C<->Rust boundary at runtime — the companion to the compile-time
   _TST_ABI_ASSERT pins (EMB-ABI32-1). */
static int demux_check(const uint8_t *ts, size_t ts_len, const uint8_t input[20]) {
    struct TstDemuxer *dmx = tst_demuxer_open();
    if (!dmx) return fail_demux("demuxer_open", 0);
    if (tst_demuxer_feed(dmx, ts, ts_len) != 0) return fail_demux("feed", 0);
    if (tst_demuxer_flush(dmx) != 0) return fail_demux("flush", 0);

    int saw_program_map = 0, saw_video_sample = 0, events = 0;
    for (;;) {
        tst_event_t ev;
        int rc = tst_demuxer_next_event(dmx, &ev);
        if (rc == TST_E_NOT_AVAILABLE) break;               /* drained */
        if (rc != 0) return fail_demux("next_event rc", (unsigned)-rc);
        if (++events > 64) return fail_demux("event flood", (unsigned)events);

        if (ev.kind == TST_EVENT_KIND_PROGRAM_MAP) {
            /* Crosses tst_event_program_map_t + tst_stream_info_t. */
            if (ev.u.program_map.program_number != 1)
                return fail_demux("pm.program_number", ev.u.program_map.program_number);
            if (ev.u.program_map.pmt_pid != 0x1000)
                return fail_demux("pm.pmt_pid", ev.u.program_map.pmt_pid);
            if (ev.u.program_map.pcr_pid != 0x1011)
                return fail_demux("pm.pcr_pid", ev.u.program_map.pcr_pid);
            if (ev.u.program_map.stream_count != 1 || !ev.u.program_map.streams)
                return fail_demux("pm.stream_count", (unsigned)ev.u.program_map.stream_count);
            const tst_stream_info_t *si = &ev.u.program_map.streams[0];
            if (si->pid != 0x1011) return fail_demux("si.pid", si->pid);
            if (si->stream_kind != TST_STREAM_KIND_VIDEO)
                return fail_demux("si.stream_kind", (unsigned)si->stream_kind);
            if (si->codec != TST_VIDEO_CODEC_H264)
                return fail_demux("si.codec", (unsigned)si->codec);
            saw_program_map = 1;
        } else if (ev.kind == TST_EVENT_KIND_SAMPLE &&
                   ev.u.sample.stream_kind == TST_STREAM_KIND_VIDEO) {
            /* Crosses tst_event_sample_t (i64 pts/dts alignment) + tst_nal_t. */
            if (ev.u.sample.pid != 0x1011) return fail_demux("s.pid", ev.u.sample.pid);
            if (ev.u.sample.codec != TST_VIDEO_CODEC_H264)
                return fail_demux("s.codec", (unsigned)ev.u.sample.codec);
            if (ev.u.sample.pts != 0)
                return fail_demux("s.pts", (unsigned)(uint64_t)ev.u.sample.pts);
            if (ev.u.sample.dts != INT64_MIN)  /* absent-DTS sentinel */
                return fail_demux("s.dts", (unsigned)(uint64_t)ev.u.sample.dts);
            if (ev.u.sample.payload_len != 20 || !ev.u.sample.payload)
                return fail_demux("s.payload_len", (unsigned)ev.u.sample.payload_len);
            if (memcmp(ev.u.sample.payload, input, 20) != 0)
                return fail_demux("s.payload bytes", ev.u.sample.payload[0]);
            if (ev.u.sample.nal_count != 1 || !ev.u.sample.nals)
                return fail_demux("s.nal_count", (unsigned)ev.u.sample.nal_count);
            const tst_nal_t *nal = &ev.u.sample.nals[0];
            if (nal->nal_type != 5) return fail_demux("nal.nal_type", nal->nal_type);
            if (nal->ref_idc_or_layer_id != 3)
                return fail_demux("nal.ref_idc", nal->ref_idc_or_layer_id);
            /* H.264 NAL views strip the start code AND the 1-byte header. */
            if (nal->payload_len != 15 || !nal->payload)
                return fail_demux("nal.payload_len", (unsigned)nal->payload_len);
            if (nal->payload[0] != 0xA5)
                return fail_demux("nal.payload[0]", nal->payload[0]);
            saw_video_sample = 1;
        }
    }
    tst_demuxer_close(dmx);
    if (!saw_program_map)  return fail_demux("missing ProgramMap event", (unsigned)events);
    if (!saw_video_sample) return fail_demux("missing video Sample event", (unsigned)events);
    printf("PASS: c_firmware demux struct-crossing (%d events)\n", events);
    return 0;
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

    if (total != (size_t)GOLDEN_LEN || memcmp(out, GOLDEN, total) != 0) {
        size_t cmp_len = total < (size_t)GOLDEN_LEN ? total : (size_t)GOLDEN_LEN;
        size_t first = cmp_len; /* default: lengths differ, no byte overlap */
        for (size_t i = 0; i < cmp_len; i++) {
            if (out[i] != GOLDEN[i]) { first = i; break; }
        }
        printf("FAIL[c_firmware] mismatch: produced %u bytes, golden %u;"
               " first mismatch at offset %u (expected 0x%02x, got 0x%02x)\n",
               (unsigned)total, (unsigned)GOLDEN_LEN,
               (unsigned)first,
               first < (size_t)GOLDEN_LEN ? (unsigned)GOLDEN[first] : 0u,
               first < total             ? (unsigned)out[first]   : 0u);
        return 1;
    }

    if (demux_check(out, total, input) != 0) return 1;

    printf("PASS: c_firmware (%u bytes)\n", (unsigned)total);
    return 0;
}
