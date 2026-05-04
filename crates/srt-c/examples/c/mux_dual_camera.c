/*
 * mux_dual_camera.c — Dual-camera (EO + IR) + KLV → MPEG-TS file.
 *
 * Build (from the srt-rust workspace root):
 *   SRT_FORCE_VENDORED=1 cargo build -p srt-c
 *   cc -I crates/srt-c/include \
 *      -L target/debug \
 *      -Wall -Werror \
 *      -o /tmp/mux_dual_camera \
 *      crates/srt-c/examples/c/mux_dual_camera.c -lsrtc
 *
 * Run:
 *   LD_LIBRARY_PATH=target/debug /tmp/mux_dual_camera
 *
 * Output:
 *   dual_camera.ts in the current directory.
 *
 * Verify:
 *   ffprobe -show_streams dual_camera.ts
 *     # Should report 2 video streams (h264) + 1 data stream.
 *   tsp -I file dual_camera.ts -P analyze
 *     # TSDuck's PSI/SI walker — confirms PMT enumerates all 3 PIDs.
 *
 * Mirrors crates/srt-core/examples/mux_dual_camera.rs (Rust).
 * The C version is more verbose because there's no RAII, and because
 * C readers may have less context about what the safe Rust wrappers
 * underneath are doing on their behalf.
 */

#include "srtc.h"
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/*
 * Synthetic Annex-B NALs — 4-byte start code (0x00 0x00 0x00 0x01)
 * followed by nal_unit_type 0x67 (SPS) and two filler bytes.
 *
 * WHY so short, and why 0x67?
 *   ffprobe and downstream demuxers identify the codec from the PMT's
 *   stream_type byte (0x1B for H.264) — NOT from parsing the NAL
 *   contents. So a tiny stub is enough for the muxer's structural
 *   roundtrip: PAT/PMT/PCR/PES are all generated correctly, and
 *   ffprobe will report "codec_name=h264" on both video streams even
 *   though this is not a decodable bitstream.
 *
 *   The EO and IR NALs differ only in the filler byte (0xAA vs 0xBB)
 *   so you can verify the muxer is routing each feed to the right PID
 *   if you hex-dump the output.
 */
static const uint8_t NAL_EO[] = { 0x00, 0x00, 0x00, 0x01, 0x67, 0xAA, 0xFF };
static const uint8_t NAL_IR[] = { 0x00, 0x00, 0x00, 0x01, 0x67, 0xBB, 0xFF };

/*
 * Minimal 17-byte ST 0601 KLV blob.
 *
 * WHY exactly 17 bytes?
 *   A conformant ST 0601 packet requires:
 *     - 16 bytes: UAS Datalink Local Set Universal Label (the UL below)
 *     -  1 byte:  BER short-form length field = 0 (zero-length value)
 *   Total: 17 bytes — the smallest legal ST 0601 KLV envelope.
 *   The zero-length value means "no tags" inside the Local Set. This
 *   is spec-conformant and is enough for the muxer to produce a
 *   well-formed KLV PES packet on the correct PID.
 *
 * WHY this particular UL?
 *   0x06 0x0E 0x2B 0x34 ... is the SMPTE/MISB UL prefix. The full 16
 *   bytes spell out the UAS Datalink Local Set key from MISB ST 0601.
 *   Any conformant KLV parser will recognize the stream as ST 0601.
 */
static const uint8_t KLV_BLOB[17] = {
    0x06, 0x0E, 0x2B, 0x34, 0x02, 0x0B, 0x01, 0x01,
    0x0E, 0x01, 0x03, 0x01, 0x01, 0x00, 0x00, 0x00,
    0x00,   /* length = 0 */
};

int main(void) {
    /*
     * ── Step 1: Build a multi-stream mux config ──────────────────────────
     *
     * srtc_mux_config_t is an opaque heap-allocated builder. You populate it
     * via the srtc_mux_config_add_* family, then hand it to srtc_muxer_open.
     * The muxer consumes (copies) the config — you own the pointer and must
     * free it yourself, but the muxer doesn't alias it after _open returns.
     */
    srtc_mux_config_t *cfg = srtc_mux_config_new();
    if (!cfg) {
        /* srtc_mux_config_new fails only on OOM. No last-error to print. */
        fprintf(stderr, "srtc_mux_config_new: out of memory\n");
        return 1;
    }

    /*
     * Register program 1. Every mux config must have at least one program
     * before streams can be added. The program handle (an ordinal) is passed
     * to subsequent stream-add calls.
     *
     * WHY program_number=1 and pmt_pid=0x1000?
     *   MPEG-TS PAT reserves program_number=0 for the NIT pointer; our data
     *   program is program 1. PMT PID 0x1000 is a widely-used convention for
     *   single-program TS — most decoders accept it without configuration.
     */
    srtc_program_handle_t prog =
        srtc_mux_config_add_program(cfg, /*program_number=*/1, /*pmt_pid=*/0x1000);
    if (prog.value == SRTC_INVALID_PROGRAM_HANDLE.value) {
        fprintf(stderr, "add_program failed: %s\n", srtc_get_last_error_str());
        srtc_mux_config_free(cfg);
        return 2;
    }

    /*
     * Add two video streams (EO visible-light, IR thermal) and one KLV stream.
     *
     * WHY these specific PID values (0x1011, 0x1021, 0x1031)?
     *   No MPEG-TS spec mandates a particular PID layout for elementary streams.
     *   The constraints are:
     *     - PIDs 0x0000–0x000F are reserved (PAT, CAT, NIT, etc.).
     *     - PID 0x1FFF is the null packet PID.
     *     - PMT PID is 0x1000 for this program.
     *   Spreading our PIDs by ≥16 (0x10 hex) makes them visually distinct in a
     *   Wireshark or TSDuck capture and avoids accidental adjacency to the PMT.
     *
     * srtc_mux_config_add_video_stream returns a srtc_video_stream_handle_t —
     * a packed uint32_t encoding (program_index, within-program stream index).
     * You MUST use handles when pushing to a multi-stream muxer: the single-
     * stream push variants (push_video, push_klv) return SRTC_E_INVALID_USAGE
     * when more than one stream of that kind is configured. Capture the handle
     * now; it remains valid for the lifetime of the muxer (and across
     * managed-sender reconnects).
     */
    srtc_video_stream_handle_t h_eo =
        srtc_mux_config_add_video_stream(cfg, prog, 0x1011, SRTC_VIDEO_CODEC_H264);
    srtc_video_stream_handle_t h_ir =
        srtc_mux_config_add_video_stream(cfg, prog, 0x1021, SRTC_VIDEO_CODEC_H264);

    /*
     * WHY SRTC_KLV_STREAM_TYPE_PRIVATE_DATA (async KLV)?
     *   There are two KLV stream types:
     *     - SRTC_KLV_STREAM_TYPE_PRIVATE_DATA (stream_type 0x06): "asynchronous"
     *       KLV. The KLV PES packet carries no PTS. Downstream parsers treat
     *       KLV as a best-effort side-channel aligned by arrival, not timestamp.
     *       This is the simpler shape — no AU cell wrapping required.
     *     - SRTC_KLV_STREAM_TYPE_SYNCHRONOUS_METADATA (stream_type 0x15):
     *       "synchronous" KLV. The PES carries a PTS, so a demuxer can
     *       pair each KLV packet with the nearest-PTS video frame.
     *       IMPORTANT: for synchronous KLV, callers MUST pre-wrap each blob
     *       via the ST 1910 AU cell wrapper BEFORE calling push_klv_to. The
     *       muxer does NOT auto-wrap.
     *   This example uses async (carries_pts=false) — the simpler shape.
     *
     * The handle returned here is a srtc_klv_stream_handle_t (also uint32_t).
     * SRTC_INVALID_STREAM_HANDLE (UINT32_MAX) signals failure; check
     * srtc_get_last_error_str() for the reason.
     */
    srtc_klv_stream_handle_t h_klv =
        srtc_mux_config_add_klv_stream(cfg, prog, 0x1031,
                                       SRTC_KLV_STREAM_TYPE_PRIVATE_DATA,
                                       /*carries_pts=*/false);

    /* Check all three handles before proceeding. */
    if (h_eo == SRTC_INVALID_STREAM_HANDLE ||
        h_ir == SRTC_INVALID_STREAM_HANDLE ||
        h_klv == SRTC_INVALID_STREAM_HANDLE) {
        fprintf(stderr, "stream-add failed: %s\n", srtc_get_last_error_str());
        srtc_mux_config_free(cfg);
        return 3;
    }

    /*
     * Pin the PCR to the EO video stream (PID 0x1011).
     *
     * WHY pin PCR to the primary video stream?
     *   PCR (Program Clock Reference) is the master wall-clock for a program.
     *   Convention: pin it to whichever stream a downstream demuxer should
     *   use as its master clock — typically the primary visible-light feed
     *   (EO), not the thermal imager (IR).
     *
     * WHY is this call "redundant but illustrative"?
     *   For multi-stream muxers, the auto-default is already "first video
     *   stream's PID." Since EO was added first (0x1011), omitting this call
     *   produces identical output. It's included here so readers of this
     *   example see the explicit form and know how to override it (e.g., if
     *   you wanted PCR driven by the IR feed instead).
     */
    srtc_mux_config_set_pcr_pid(cfg, prog, 0x1011);

    /*
     * ── Step 2: Open the muxer ────────────────────────────────────────────
     *
     * srtc_muxer_open copies what it needs from cfg — after this returns,
     * cfg can be freed immediately. Keeping cfg alive longer is harmless but
     * misleading; free it right after _open to signal "config is consumed."
     *
     * On success, srtc_muxer_open returns an opaque heap-allocated muxer.
     * On failure (e.g., TooManyVideoStreams — cap is 16), it returns NULL
     * and sets the thread-local last-error.
     */
    srtc_muxer_t *mux = srtc_muxer_open(cfg);
    srtc_mux_config_free(cfg);
    cfg = NULL;  /* prevent accidental use after free */

    if (!mux) {
        fprintf(stderr, "srtc_muxer_open failed: %s\n", srtc_get_last_error_str());
        return 4;
    }

    /*
     * ── Step 3: Open the output file ─────────────────────────────────────
     */
    FILE *out = fopen("dual_camera.ts", "wb");
    if (!out) {
        perror("fopen dual_camera.ts");
        srtc_muxer_close(mux);
        return 4;
    }

    /*
     * ── Step 4: Push 30 frames and drain ─────────────────────────────────
     *
     * WHY 3000 ticks per frame?
     *   MPEG-TS PTS/DTS/PCR use a 90 kHz clock (90,000 ticks per second).
     *   At 30 fps: 90,000 / 30 = 3,000 ticks per frame.
     *   Common values:
     *     25 fps → 3,600 ticks/frame
     *     29.97 fps → 3,003 ticks/frame (NTSC drop-frame)
     *     30 fps → 3,000 ticks/frame   ← this example
     *
     * WHY a drain loop inside the frame loop?
     *   srtc_muxer_pull has internal buffering (default ~10,000 TS packets
     *   ≈ 600 ms at 25 Mbps). On a short 30-frame run the buffer won't
     *   overflow, but the canonical pattern for file output is: push one
     *   frame, drain everything ready, repeat. This avoids building up a
     *   large in-memory backlog on longer runs (e.g., a 2-hour recording).
     *   srtc_muxer_pull returns 0 when no aligned data is ready — that's
     *   the normal "buffer not yet full" signal, not an error.
     *
     * WHY check n % 188 == 0 and buf[0] == 0x47?
     *   Every MPEG-TS packet is exactly 188 bytes. The muxer only emits
     *   complete packets, so the returned byte count must be a multiple of
     *   188. The first byte of every TS packet is the sync byte 0x47.
     *   These two checks are cheap structural sanity guards — if either
     *   fails, something is seriously wrong (bug in the muxer, or a
     *   too-small output buffer that truncated the pull).
     *
     * WHY 188 * 64 for the drain buffer?
     *   64 TS packets = 12,032 bytes. The muxer emits PAT + PMT + PCR +
     *   PES packets; a burst of ~10–20 packets per frame at this bitrate
     *   is typical. 64 is generous enough that one pull call usually drains
     *   the whole burst, avoiding multiple iterations of the inner while loop.
     */
    uint8_t buf[188 * 64];

    for (int i = 0; i < 30; i++) {
        int64_t pts = (int64_t)i * 3000;
        bool key = (i == 0);  /* keyframe on the first frame only */

        /* Push EO video. */
        if (srtc_muxer_push_video_to(mux, h_eo, NAL_EO, sizeof(NAL_EO), pts, key) != 0) {
            fprintf(stderr, "EO push[%d] failed: %s\n", i, srtc_get_last_error_str());
            goto fail;
        }

        /* Push IR video. Same PTS — both cameras are frame-sync'd. */
        if (srtc_muxer_push_video_to(mux, h_ir, NAL_IR, sizeof(NAL_IR), pts, key) != 0) {
            fprintf(stderr, "IR push[%d] failed: %s\n", i, srtc_get_last_error_str());
            goto fail;
        }

        /* Push KLV. pts is passed but ignored by the muxer for async KLV
         * (carries_pts=false). It still influences PSI/PCR cadence scheduling
         * inside the muxer, so passing the real frame PTS is correct. */
        if (srtc_muxer_push_klv_to(mux, h_klv, KLV_BLOB, sizeof(KLV_BLOB), pts) != 0) {
            fprintf(stderr, "KLV push[%d] failed: %s\n", i, srtc_get_last_error_str());
            goto fail;
        }

        /* Drain all ready TS packets to the output file. */
        size_t n;
        while ((n = srtc_muxer_pull(mux, buf, sizeof(buf))) > 0) {
            /* Structural sanity: every pull must be a whole-packet multiple. */
            if ((n % 188) != 0 || buf[0] != 0x47) {
                fprintf(stderr,
                        "alignment check failed at frame %d: "
                        "n=%zu first=0x%02x\n",
                        i, n, (unsigned)buf[0]);
                goto fail;
            }
            fwrite(buf, 1, n, out);
        }
    }

    fclose(out);
    srtc_muxer_close(mux);

    printf("Wrote dual_camera.ts\n");
    printf("Try: ffprobe -show_streams dual_camera.ts\n");
    return 0;

fail:
    /*
     * WHY close muxer before freeing cfg?
     *   cfg was already freed right after _open above. The only live handle
     *   here is the muxer itself. Close it so libsrtc can release its own
     *   internal state cleanly; fclose flushes/releases the file descriptor.
     *   Order: close file first (safe to have partial output) then muxer —
     *   or either order is fine here since the muxer doesn't hold a file ref.
     */
    fclose(out);
    srtc_muxer_close(mux);
    return 5;
}
