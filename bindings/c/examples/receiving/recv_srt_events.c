/*
 * recv_srt_events.c — open the MANAGED (auto-reconnecting) SRT demux
 * receiver against a URL taken from argv[1] (caller or listener mode,
 * URL-driven — see below), walk every `tst_event_t` kind (including the
 * reconnect boundary marker), decode ST 0601 KLV inline, and shut down
 * via a lock-free signal-path cancel.
 *
 * Why this example:
 *   This is the BEHAVIORAL REFERENCE for the Apple/Swift wrapper — the
 *   shape a Swift `AsyncStream`-backed receiver is written against.
 *   Every choice below (which handle family, how cancel is sequenced,
 *   how KLV decode failure is handled, how the reconnect marker is
 *   surfaced) is a choice the Swift layer must reproduce, so it's
 *   documented as a decision, not just a call.
 *
 *   Diff from `recv_demux_to_console.c` (the older flagship event-walk
 *   example), which this example supersedes for the managed+caller+
 *   KLV-decode case:
 *     - `recv_demux_to_console.c` opens the PLAIN `tst_demux_receiver_t`
 *       via `_open_listener` with a hardcoded `srt://:7000` and ignores
 *       argv entirely. This example takes the URL from argv[1] and opens
 *       the MANAGED family (`tst_managed_demux_receiver_t`), which
 *       transparently retries the whole connect-through-first-byte
 *       sequence on transport failure per the configured
 *       `tst_reconnect_policy_t` (NULL here = library defaults).
 *     - `recv_demux_to_console.c`'s `kind_name()` switch has NO case for
 *       `TST_EVENT_KIND_RECONNECT_DISCONTINUITY` (kind 6) — it falls
 *       through to `default: "????"`. That event kind is emitted ONLY by
 *       the managed family (a plain `tst_demux_receiver_t` never reconnects,
 *       so it never fires), which is exactly why the older example never
 *       needed a case for it. This example is managed-only, so handling
 *       kind 6 correctly is the whole point, not an afterthought.
 *     - `recv_demux_to_console.c` prints raw struct fields for METADATA
 *       events (kind/pid/len/seq) but does not decode the KLV payload.
 *       This example decodes MISB ST 0601 inline via the Task-7 typed
 *       surface (`tst_st0601_decode` / `tst_st0601_geometry`) so callers
 *       can see the (value, state) contract that surface exposes.
 *
 * Caller mode vs listener mode — both from ONE code path:
 *   `tst_managed_demux_receiver_open`'s URL-driven mode dispatch matches
 *   `tst_demux_receiver_open`: a bare `srt://host:port` connects OUT as a
 *   caller (the shape a ground station uses against a camera's SRT
 *   listener); appending `?mode=listener` (e.g. `srt://:9000?mode=listener`
 *   or `srt://0.0.0.0:9000?mode=listener`) instead BINDS and waits for a
 *   peer to connect IN. There is no separate `_open_listener` call needed
 *   here — the query string alone selects the mode, so this one example
 *   covers both topologies a real deployment uses (ground-station-dials-
 *   camera vs camera-dials-ground-station).
 *
 * How to run:
 *   Both sender examples referenced below (`mux_synthetic_srt.c`,
 *   `poll_socket_stats.c`) only ever dial OUT as an SRT caller — neither
 *   has a listener mode — so in every recipe here THIS example is the
 *   listener the sender connects into.
 *
 *   Recipe 1 — quick event-shape demo (PROGRAM_MAP / SAMPLE / METADATA),
 *   ~0.4s total sender lifetime, DO NOT try to Ctrl-C mid-run (see the
 *   GOTCHA below — the window is too short to reliably land inside it):
 *     Terminal A:
 *       SRT_FORCE_VENDORED=1 cargo build -p tst-c --features srt
 *       cc -I bindings/c/include -L target/debug -Wall -Werror \
 *          -o /tmp/recv_srt_events \
 *          bindings/c/examples/receiving/recv_srt_events.c -ltstrans
 *       LD_LIBRARY_PATH=target/debug /tmp/recv_srt_events \
 *           'srt://:9000?mode=listener'
 *     Terminal B:
 *       cc -I bindings/c/include -L target/debug -Wall -Werror \
 *          -o /tmp/mux_synthetic_srt \
 *          bindings/c/examples/muxing/mux_synthetic_srt.c -ltstrans
 *       LD_LIBRARY_PATH=target/debug /tmp/mux_synthetic_srt 127.0.0.1:9000
 *     Verified output (5 AUs sent, 4 events land before the sender's
 *     linger/close races the demuxer's last read — this is normal, not
 *     a drop; the remaining AUs are still in flight when the process
 *     tree gets torn down at the end of this short-lived demo):
 *       [PMT ] program=1 pcr_pid=0x1011 pmt_pid=0x1000 streams=2 klv_links=1
 *       [META] klv decode failed bytes=33: buffer truncated at offset 17: needed 3 bytes, have 0
 *       [SMPL] pts=0 dts=- key=1 codec=H264 size=500 nals=1
 *       [META] klv decode failed bytes=33: buffer truncated at offset 0: needed 2 bytes, have 1
 *     Stop the receiver afterward with `kill -9` (or a second Ctrl-C
 *     press) — see the GOTCHA note for why a single Ctrl-C may not land
 *     promptly once the sender has already disconnected.
 *
 *   Recipe 2 — reliable Ctrl-C / cancel-path demo (a 5-second continuous
 *   stream gives a wide, easy-to-hit window to cancel while GENUINELY
 *   blocked in `_recv_event` on a still-live connection — the scenario
 *   this example's SIGINT handling is built around):
 *     Terminal A: same `recv_srt_events 'srt://:9000?mode=listener'` as above.
 *     Terminal B:
 *       cc -I bindings/c/include -L target/debug -Wall -Werror \
 *          -o /tmp/poll_socket_stats \
 *          bindings/c/examples/operations/poll_socket_stats.c -ltstrans
 *       LD_LIBRARY_PATH=target/debug /tmp/poll_socket_stats srt://127.0.0.1:9000
 *     Press Ctrl-C on Terminal A any time in the first ~4.5s. Verified
 *     across repeated runs: on the order of 50-60 SAMPLE/PROGRAM_MAP
 *     events received (exact count depends on exactly when you press
 *     Ctrl-C — not a fixed number), clean exit, printing
 *     "receiver cancelled (SIGINT); <n> events received" then
 *     "end reason: CANCELLED".
 *
 *   Caller-mode variant of either recipe (swap who binds vs dials — point
 *   this example at a real listener instead, e.g. a camera or
 *   `srt-live-transmit srt://:9000?mode=listener ...`):
 *     LD_LIBRARY_PATH=target/debug /tmp/recv_srt_events srt://127.0.0.1:9000
 *
 *   GOTCHA — observed this session, not (yet) a documented library
 *   guarantee either way: if a peer disconnects and this listener-mode
 *   receiver goes looking for a NEW one (managed auto-reconnect), Ctrl-C
 *   during THAT specific window does not exit promptly. tstrans.h already
 *   documents that a listener's `accept()` wait is unbounded; what this
 *   session additionally observed is that `_cancel` does not appear to
 *   unblock that particular wait either — there is no live data socket
 *   yet for it to close. A second signal (`kill -9`) is the practical way
 *   out if you land in this window. This is a real, reproducible
 *   interaction worth a closer look outside the scope of this example —
 *   it does NOT affect the documented, and verified above (Recipe 2),
 *   "cancel while genuinely blocked in `_recv_event` on an established
 *   connection" contract this example's SIGINT handling is built around.
 *
 *   NOTE on `mux_synthetic_srt.c`'s KLV (Recipe 1 only): its `make_klv()`
 *   writes the 16-byte ST 0601 Universal Label + a BER length byte, but
 *   the 16 "value" bytes are just the frame sequence number repeated —
 *   NOT a real tag/length/value chain, and there's no trailing checksum
 *   tag (every real ST 0601 record ends with tag 1, a 2-byte BER-OID
 *   checksum). `tst_st0601_decode` on that payload hits the NULL /
 *   hard-decode-failure path below (verified above) — that's fine, it
 *   exercises exactly the "gracefully handle a non-decodable payload"
 *   branch this example is required to have. Point this example at a
 *   real MISB-conformant KLV source (a gimbaled-platform camera, or
 *   `tests/tools/gen_synthetic_fixtures`-produced fixtures muxed via a
 *   Rust sender) to see populated lat/lon/alt/hdg.
 *
 * Build (from the ts-transformer workspace root):
 *   SRT_FORCE_VENDORED=1 cargo build -p tst-c --features srt
 *   cc -I bindings/c/include \
 *      -L target/debug \
 *      -Wl,-rpath,target/debug \
 *      -Wall -Wextra -Werror \
 *      -o /tmp/recv_srt_events \
 *      bindings/c/examples/receiving/recv_srt_events.c -ltstrans
 *
 * Closest Rust analog: examples/receiving/srt_recv_typed.rs (plain,
 *   non-managed event walk — its comment at the ReconnectDiscontinuity
 *   match arm is the Rust-side explanation of the same kind-6 boundary).
 *   No Rust example currently exercises `ManagedDemuxReceiver` + typed
 *   ST 0601 decode together; this C example is the first.
 */

#include "tstrans.h"

#if !defined(TST_HAS_SRT) || TST_HAS_SRT == 0
#error "This example requires TST_HAS_SRT. Rebuild tst-c with the srt cargo feature enabled."
#endif

#include <inttypes.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* ── SIGINT state ─────────────────────────────────────────────────────────
 *
 * Same shape as `recv_rtp.c`'s cancel pattern, applied to the managed SRT
 * family. `tst_managed_demux_receiver_cancel` is documented (see
 * tstrans.h) as a side-channel operation — no Mutex acquisition, just a
 * socket close — which is what makes calling it DIRECTLY from the signal
 * handler body reasonable here, unlike an arbitrary tst-c entry point.
 * ---------------------------------------------------------------------- */
static volatile tst_managed_demux_receiver_t *g_receiver = NULL;
static volatile sig_atomic_t g_shutdown_requested = 0;

static void on_sigint(int sig) {
    (void) sig;
    g_shutdown_requested = 1;

    /*
     * Why call cancel from the signal handler instead of just setting a
     * flag the main loop polls?
     *   `tst_managed_demux_receiver_recv_event` blocks indefinitely (no
     *   per-call timeout unless `?x-recvtimeout=<ms>` was set on the URL).
     *   A flag-only handler would leave the blocked thread parked until
     *   the NEXT packet arrives, which may be never. `_cancel` closes the
     *   underlying socket, which unblocks a thread parked in `_recv_event`
     *   within about one libsrt I/O cycle (roughly 3-10 ms) regardless of
     *   whether more data ever shows up.
     *
     *   `_cancel` is safe to call here specifically because it is
     *   lock-free and callable from any thread/context — it does not
     *   acquire the data-path mutex that a concurrent `_recv_event` may be
     *   holding, and it performs no heap allocation. `_close`, by
     *   contrast, DOES acquire that mutex and then frees the handle — it
     *   is NOT safe to call from here (or from any thread racing a
     *   blocked `_recv_event`): see the cancel→close ordering note above
     *   `main()`'s cleanup step below.
     *
     *   The cast drops `volatile`; safe because `_cancel` is called
     *   exactly once per process lifetime from this handler (SIGINT is
     *   typically delivered once during interactive Ctrl-C use) and the
     *   handle's lifetime spans all of `main()`.
     */
    tst_managed_demux_receiver_cancel((tst_managed_demux_receiver_t *) g_receiver);
}

/* ── small formatting helpers ────────────────────────────────────────────── */

static const char *event_tag(int kind) {
    switch (kind) {
        case TST_EVENT_KIND_PROGRAM_MAP:             return "PMT ";
        case TST_EVENT_KIND_SAMPLE:                  return "SMPL";
        case TST_EVENT_KIND_METADATA:                return "META";
        case TST_EVENT_KIND_DISCONTINUITY:           return "DISC";
        case TST_EVENT_KIND_NON_CONFORMANT:           return "NONC";
        case TST_EVENT_KIND_RECONNECT_DISCONTINUITY: return "RCON";
        default:                                      return "????";
    }
}

/*
 * `codec` is only textually named for video samples — per tstrans.h's
 * `tst_stream_info_t` doc, `codec` is keyed by `stream_kind`
 * (`TST_VIDEO_CODEC_*` / `TST_AUDIO_CODEC_*` / `TST_SUBTITLE_CODEC_*`, or
 * -1 for KLV/Unknown streams). This example's SAMPLE line format is
 * video-focused, so audio/subtitle samples fall back to the numeric id.
 */
static const char *video_codec_name(int codec) {
    switch (codec) {
        case TST_VIDEO_CODEC_H264: return "H264";
        case TST_VIDEO_CODEC_H265: return "H265";
        case TST_VIDEO_CODEC_H266: return "H266";
        case TST_VIDEO_CODEC_AV1:  return "AV1";
        default:                   return NULL;
    }
}

static const char *end_reason_name(enum tst_stream_end_reason r) {
    switch (r) {
        case TST_STREAM_END_REASON_NONE:            return "NONE";
        case TST_STREAM_END_REASON_CLEAN_TEARDOWN:   return "CLEAN_TEARDOWN";
        case TST_STREAM_END_REASON_SESSION_EXPIRED:  return "SESSION_EXPIRED";
        case TST_STREAM_END_REASON_KEEPALIVE_FAILED: return "KEEPALIVE_FAILED";
        case TST_STREAM_END_REASON_TRANSPORT_FAILED: return "TRANSPORT_FAILED";
        case TST_STREAM_END_REASON_PROTOCOL_ERROR:   return "PROTOCOL_ERROR";
        case TST_STREAM_END_REASON_CANCELLED:        return "CANCELLED";
        default:                                     return "UNKNOWN";
    }
}

/* Format an ST 0601 geometry double field: the value if `state` is
 * PRESENT, "-" for every other state (Absent / Sentinel / ImapbSpecial —
 * `tst_st0601_geometry` never returns WrongType, that's a per-tag-getter
 * only code, see tstrans.h's `TstSt0601FieldState` doc). Printing "-"
 * instead of a bit pattern like 0.0 matters: 0.0 is a valid latitude
 * (the equator) — collapsing "absent" into "zero" would silently corrupt
 * a real reading, so the tri-state (in practice five-state) contract
 * exists specifically so callers never have to guess. */
static void fmt_geo_f64(char *buf, size_t buflen, uint8_t state, double val) {
    if (state == TST_ST0601_FIELD_STATE_PRESENT) {
        snprintf(buf, buflen, "%.6f", val);
    } else {
        snprintf(buf, buflen, "-");
    }
}

static void fmt_geo_u64(char *buf, size_t buflen, uint8_t state, uint64_t val) {
    if (state == TST_ST0601_FIELD_STATE_PRESENT) {
        snprintf(buf, buflen, "%" PRIu64, val);
    } else {
        snprintf(buf, buflen, "-");
    }
}

/* ── per-event-kind printers ─────────────────────────────────────────────── */

static void print_program_map(const tst_event_t *ev) {
    fprintf(stdout,
            "[%s] program=%u pcr_pid=0x%04x pmt_pid=0x%04x streams=%zu klv_links=%zu\n",
            event_tag(ev->kind),
            ev->u.program_map.program_number,
            ev->u.program_map.pcr_pid,
            ev->u.program_map.pmt_pid,
            ev->u.program_map.stream_count,
            ev->u.program_map.klv_link_count);
}

static void print_sample(const tst_event_t *ev) {
    /*
     * dts uses `INT64_MIN` as the "not present" sentinel (a NAL-shaped
     * codec without B-frames, or any stream whose PES header carried
     * PTS but not DTS) — see `bindings/c/core/src/event.rs`'s
     * `dts: i64, // INT64_MIN if absent` field comment. `pts`, by
     * contrast, has NO such sentinel at this layer: `DemuxEvent::Sample`
     * carries a non-optional `Pts90khz`, so it is always a real value
     * here and needs no "-" branch (unlike the RTP-side comment you may
     * have seen elsewhere claiming `pts==INT64_MIN` — that was about a
     * different receiver family; verify against the .rs source per this
     * project's binding-canonical-workflow convention rather than
     * trusting a comment in a sibling example).
     */
    char dts_buf[24];
    if (ev->u.sample.dts == INT64_MIN) {
        snprintf(dts_buf, sizeof(dts_buf), "-");
    } else {
        snprintf(dts_buf, sizeof(dts_buf), "%" PRId64, ev->u.sample.dts);
    }

    /*
     * `key` comes from `random_access_indicator` — the MPEG-TS
     * ADAPTATION-FIELD bit (ISO/IEC 13818-1 §2.4.3.4, bit 0x40) the
     * MUXER stamped on the PES_start TS packet of this access unit. It
     * is a STREAM-LEVEL signal the sender asserts, not something the
     * demuxer derives by inspecting NAL contents — a muxer could
     * (incorrectly) set it on a non-IDR AU, or clear it on a real IDR.
     * For H.264, a consumer that wants a bitstream-derived cross-check
     * can instead scan `ev->u.sample.nals[i].nal_type == 5` (IDR slice
     * per Table 7-1) among the parsed NAL views — the two signals
     * usually agree for well-formed encoders but are not the same fact.
     */
    char codec_buf[8];
    const char *cname = (ev->u.sample.stream_kind == TST_STREAM_KIND_VIDEO)
                             ? video_codec_name(ev->u.sample.codec)
                             : NULL;
    if (cname) {
        snprintf(codec_buf, sizeof(codec_buf), "%s", cname);
    } else {
        snprintf(codec_buf, sizeof(codec_buf), "%d", ev->u.sample.codec);
    }

    fprintf(stdout,
            "[%s] pts=%" PRId64 " dts=%s key=%u codec=%s size=%zu nals=%zu\n",
            event_tag(ev->kind),
            ev->u.sample.pts,
            dts_buf,
            (unsigned) ev->u.sample.random_access_indicator,
            codec_buf,
            ev->u.sample.payload_len,
            ev->u.sample.nal_count);
}

static void print_metadata(const tst_event_t *ev) {
    const struct TstEventMetadata *m = &ev->u.metadata;

    /* Only SyncAuCell / Async are KLV kinds; Unknown (a metadata stream
     * type this build doesn't classify) has no ST 0601 structure to try
     * decoding — print a generic line instead of feeding garbage bytes
     * into the decoder. */
    if (m->metadata_kind != TST_METADATA_KIND_KLV_SYNC_AU_CELL &&
        m->metadata_kind != TST_METADATA_KIND_KLV_ASYNC) {
        fprintf(stdout,
                "[%s] non-KLV metadata pid=0x%04x kind=%d bytes=%zu\n",
                event_tag(ev->kind),
                m->pid,
                m->metadata_kind,
                m->payload_len);
        return;
    }

    /*
     * `tst_st0601_decode` takes the raw KLV Local Set bytes as pulled
     * straight off this event — `m->payload` is already stripped of the
     * 5-byte sync-metadata AU-cell wrapper by the demuxer (H.222.0
     * §2.12.4.2), so no caller-side unwrap is needed here.
     *
     * Returns NULL on a HARD structural failure: truncated buffer,
     * malformed BER length/tag, checksum mismatch, or a universal label
     * this isn't ST 0601 at all. A malformed-but-plausible record (e.g.
     * an unrecognized tag mixed with valid ones) still decodes — ST
     * 0601's lenient-decode contract collects per-tag errors rather than
     * failing the whole record, so NULL specifically means "could not
     * even walk the TLV chain", not "some field looked odd".
     */
    struct tst_st0601_t *rec = tst_st0601_decode(m->payload, m->payload_len);
    if (!rec) {
        fprintf(stdout,
                "[%s] klv decode failed bytes=%zu: %s\n",
                event_tag(ev->kind),
                m->payload_len,
                tst_get_last_error_str());
        return;
    }

    /*
     * `tst_st0601_geometry` is the curated one-call summary (24 tags)
     * rather than 24 individual `tst_st0601_get_f64`/`_get_u64` round
     * trips. Every value field is paired with a `..._state` byte — read
     * the value ONLY when its state is `TST_ST0601_FIELD_STATE_PRESENT`
     * (0); the other three states this getter can produce are `Absent`
     * (tag never on the wire), `Sentinel` (tag present but carrying the
     * MISB spec's `INT_MIN`-style absent-value marker), and
     * `ImapbSpecial` (an ST 1201.5 IMAPB special: ±infinity / NaN /
     * BelowMin / AboveMax) — all three leave the paired value at 0/0.0,
     * which is why `fmt_geo_*` below gates on the state byte instead of
     * trusting the number.
     */
    struct tst_st0601_geometry_t geo;
    memset(&geo, 0, sizeof(geo));
    int rc = tst_st0601_geometry(rec, &geo);

    /* Always free — even on a geometry-getter failure — before any
     * return path. `tst_st0601_free` is the ONLY teardown for this
     * handle (there's no separate `_close`); a leaked handle here is a
     * leaked KLV-record arena per METADATA event received. */
    if (rc != 0) {
        fprintf(stdout,
                "[%s] klv geometry query failed (rc=%d): %s\n",
                event_tag(ev->kind),
                rc,
                tst_get_last_error_str());
        tst_st0601_free(rec);
        return;
    }

    char ts[24], lat[24], lon[24], alt[24], hdg[24], hfov[24];
    fmt_geo_u64(ts, sizeof(ts), geo.timestamp_state, geo.timestamp_us);
    fmt_geo_f64(lat, sizeof(lat), geo.sensor_lat_state, geo.sensor_lat_deg);
    fmt_geo_f64(lon, sizeof(lon), geo.sensor_lon_state, geo.sensor_lon_deg);
    fmt_geo_f64(alt, sizeof(alt), geo.sensor_alt_state, geo.sensor_alt_m);
    fmt_geo_f64(hdg, sizeof(hdg), geo.platform_heading_state, geo.platform_heading_deg);
    fmt_geo_f64(hfov, sizeof(hfov), geo.sensor_hfov_state, geo.sensor_hfov_deg);

    fprintf(stdout,
            "[%s] klv ts=%s lat=%s lon=%s alt=%s hdg=%s hfov=%s bytes=%zu\n",
            event_tag(ev->kind),
            ts, lat, lon, alt, hdg, hfov,
            m->payload_len);

    tst_st0601_free(rec);
}

static void print_discontinuity(const tst_event_t *ev) {
    /* CC (continuity counter) gap on a PID — usually dropped packets on
     * a lossy link. cc_expected/cc_observed are the predicted vs actual
     * 4-bit modular counter values; see recv_rtp.c's fuller discussion
     * of the per-discontinuity-kind field layout. */
    fprintf(stdout,
            "[%s] pid=0x%04x kind=%d cc=%u->%u\n",
            event_tag(ev->kind),
            ev->u.discontinuity.pid,
            ev->u.discontinuity.discontinuity_kind,
            ev->u.discontinuity.cc_expected,
            ev->u.discontinuity.cc_observed);
}

static void print_nonconformant(const tst_event_t *ev) {
    /* Advisory spec-deviation diagnostic — the demuxer continues past
     * it (CFI-tolerant mode is the config default). `detail` is a
     * library-owned static string, never NULL-unsafe to print, but can
     * itself be NULL for issue codes with no per-instance text. */
    fprintf(stdout,
            "[%s] pid=0x%04x issue=%d detail=%s\n",
            event_tag(ev->kind),
            ev->u.nonconformant.pid,
            ev->u.nonconformant.issue_code,
            ev->u.nonconformant.detail ? ev->u.nonconformant.detail : "(none)");
}

static void print_reconnect_discontinuity(const tst_event_t *ev) {
    /*
     * THIS is the event the managed family exists to emit, and the one
     * `recv_demux_to_console.c` cannot — it's only ever produced by
     * `tst_managed_demux_receiver_*` after an underlying transport
     * failure and successful reconnect, and its `TstEventBody` union
     * carries NO per-kind payload (zero-initialized `u`; the kind value
     * alone is the whole event).
     *
     * Why a real player MUST act on this, not just log it: a reconnect
     * means the receiver got a brand-new SRT connection. Everything the
     * OLD connection had taught the demuxer — PAT/PMT (PSI) state, which
     * PIDs carry which codec, in-flight PES reassembly state, the last
     * seen continuity counter per PID — is gone. The fresh connection
     * starts from a fresh TS Syncer and a fresh PSI walk. A consumer
     * that keeps feeding the OLD SPS/PPS or decoder parameters into its
     * video decoder after this point will produce corrupted frames or a
     * hard decoder error, because the byte stream on the wire restarted
     * from scratch without warning at the transport layer.
     *
     * The correct reaction: FLUSH the decoder (drop any in-flight AU,
     * discard cached SPS/PPS/codec params) and wait for the next
     * TST_EVENT_KIND_PROGRAM_MAP event to rebuild stream state before
     * resuming playback/recording. Treat it like a new stream start,
     * not a glitch in the current one.
     */
    fprintf(stdout,
            "[%s] RECONNECT — demux/PSI/PES state was reset; flush decoder"
            " and wait for the next PMT before resuming\n",
            event_tag(ev->kind));
}

/* ── main ─────────────────────────────────────────────────────────────────── */

static void print_usage(const char *prog) {
    fprintf(stderr,
            "Usage: %s <srt-url>\n"
            "\n"
            "  srt-url  Caller mode (default, connect OUT):\n"
            "             srt://<host>:<port>\n"
            "           Listener mode (BIND and wait for a peer IN):\n"
            "             srt://:<port>?mode=listener\n"
            "             srt://0.0.0.0:<port>?mode=listener\n"
            "\n"
            "Press Ctrl-C to stop cleanly.\n",
            prog);
}

int main(int argc, char **argv) {
    if (argc != 2) {
        print_usage(argv[0]);
        return 1;
    }
    const char *url = argv[1];

    /* Install the handler before opening the receiver — there must be
     * no window where Ctrl-C can't reach a cancel. signal() (not
     * sigaction) matches recv_rtp.c's convention for this crate's C
     * examples; its one-shot-reset-to-SIG_DFL quirk on some platforms
     * doesn't matter here because g_shutdown_requested plus the single
     * `_cancel` call are all we need. */
    signal(SIGINT, on_sigint);

    /*
     * `policy = NULL` selects the library's default `tst_reconnect_policy_t`
     * (exponential backoff, a bounded attempt budget, `ReconnectMode::Blocking`
     * on the receive path — `ReconnectMode::Background` is warn-and-ignored
     * for receivers per tstrans.h). Build one via `tst_reconnect_policy_new`
     * + `tst_reconnect_policy_set_*` if you need to tune backoff/attempts;
     * this example doesn't need to.
     *
     * One call handles BOTH caller and listener mode — see the header
     * comment's "Caller mode vs listener mode" section for why there's no
     * separate `_open_listener` branch here.
     */
    tst_managed_demux_receiver_t *rx = tst_managed_demux_receiver_open(url, NULL);
    if (!rx) {
        fprintf(stderr,
                "tst_managed_demux_receiver_open(\"%s\") failed: %s\n",
                url,
                tst_get_last_error_str());
        return 1;
    }

    /* Publish for the signal handler. Not mutex-guarded: SIGINT cannot
     * arrive before this store retires in practice (the handler was
     * installed above, but the process has not yet received any signal),
     * and `volatile` stops the compiler from hoisting the handler's read
     * across it. */
    g_receiver = rx;

    fprintf(stderr, "opened: %s\n", url);
    fprintf(stderr, "waiting for events. Press Ctrl-C to stop.\n");

    /* Zero-init so any union field a given kind doesn't touch reads as
     * predictable zero bytes rather than stack garbage. */
    tst_event_t ev = {0};
    uint64_t total_events = 0;
    int exit_code = 0;

    for (;;) {
        /*
         * Blocks until one event is ready, the handle is cancelled, or
         * the managed transport gives up on reconnecting. Pointer fields
         * on `ev` (payload, nals, detail, ...) borrow from the
         * receiver's internal arena and are valid ONLY until the next
         * `_recv_event` or `_close` call on this same handle — this
         * example uses them immediately inside each print_* call and
         * never retains them past that point, which is why no memcpy
         * appears here. A consumer that needs a payload to outlive the
         * next iteration (e.g. queuing frames for an async decoder,
         * which is exactly the Swift wrapper's use case) MUST copy the
         * bytes out before looping back to `_recv_event`.
         */
        int rc = tst_managed_demux_receiver_recv_event(rx, &ev);

        if (rc == 0) {
            total_events++;
            switch (ev.kind) {
                case TST_EVENT_KIND_PROGRAM_MAP:
                    print_program_map(&ev);
                    break;
                case TST_EVENT_KIND_SAMPLE:
                    print_sample(&ev);
                    break;
                case TST_EVENT_KIND_METADATA:
                    print_metadata(&ev);
                    break;
                case TST_EVENT_KIND_DISCONTINUITY:
                    print_discontinuity(&ev);
                    break;
                case TST_EVENT_KIND_NON_CONFORMANT:
                    print_nonconformant(&ev);
                    break;
                case TST_EVENT_KIND_RECONNECT_DISCONTINUITY:
                    print_reconnect_discontinuity(&ev);
                    break;
                default:
                    /* Forward-compat: a future minor ABI bump could add a
                     * kind this build doesn't know about. Log and keep
                     * going rather than aborting the stream over it. */
                    fprintf(stderr, "[????] unknown event kind %d — ignoring\n", ev.kind);
                    break;
            }
            fflush(stdout);
            continue;
        }

        /*
         * Loop-exit paths. Unlike the plain `tst_demux_receiver_*`
         * family, the MANAGED receiver folds two different real-world
         * outcomes into the same `TST_E_END_OF_STREAM` code: a graceful
         * peer close, AND the reconnect policy exhausting its attempt
         * budget after repeated failures (libsrt cannot distinguish
         * "peer hung up" from "peer never came back" at this layer, so
         * `ManagedRecvTransport` deliberately leaves both in the same
         * terminal state). That's exactly why the end-reason query
         * after this loop exists — see the cleanup section below.
         */
        if (rc == TST_E_END_OF_STREAM) {
            fprintf(stderr,
                    "\nstream ended; %" PRIu64 " events received\n",
                    total_events);
            break;
        }
        if (rc == TST_E_CLOSED) {
            fprintf(stderr,
                    "\nreceiver cancelled%s; %" PRIu64 " events received\n",
                    g_shutdown_requested ? " (SIGINT)" : "",
                    total_events);
            break;
        }

        /* Any other negative code is a real, unrecoverable error (not a
         * lifecycle outcome) — e.g. TST_E_INVALID_TS on a catastrophic
         * demux failure. Record it and fall through to the same cleanup
         * path as every other exit so the end-reason query and _close
         * still run — an early `return` here would skip both. */
        fprintf(stderr,
                "\nrecv_event failed (rc=%d): %s\n",
                rc,
                tst_get_last_error_str());
        exit_code = 2;
        break;
    }

    /*
     * ── Cleanup: query end-reason, THEN close. Order matters both ways.
     *
     * end-reason BEFORE close: `tst_managed_demux_receiver_end_reason`
     * reads a side-channel handle captured at open time. After `_close`
     * the whole handle is freed, and calling ANY `tst_managed_demux_receiver_*`
     * function on it — including this getter — is a use-after-free the
     * caller must avoid. So this call must happen while `rx` is still
     * open, which is exactly here, right after the loop and before
     * `_close`.
     *
     * cancel-then-close, not close-from-the-handler: `_cancel` (called
     * from `on_sigint` above) only closes the underlying socket and is
     * safe to race a blocked `_recv_event` — that's the whole point of
     * it being lock-free. `_close`, by contrast, acquires the data-path
     * mutex and then frees the allocation; calling it while another
     * thread might still be inside `_recv_event` is a use-after-free
     * race. The safe sequence, which this file follows end to end, is:
     * signal handler calls `_cancel` (async-signal-path, any thread) →
     * the blocked `_recv_event` call in the loop above wakes with
     * `TST_E_CLOSED` and the loop `break`s → only THEN, back on the main
     * thread that owns the handle, do we read end-reason and `_close`.
     * `_close` is never called from the signal handler.
     */
    enum tst_stream_end_reason reason = TST_STREAM_END_REASON_NONE;
    int er_rc = tst_managed_demux_receiver_end_reason(rx, &reason);
    if (er_rc == 0) {
        /*
         * On this SRT recv path you will only ever see three of the six
         * non-NONE variants: CLEAN_TEARDOWN (peer closed gracefully, or
         * we cancelled after already draining a clean close),
         * TRANSPORT_FAILED (the reconnect policy gave up — the peer
         * never came back), or CANCELLED (this process's `_cancel` won
         * the race, i.e. the SIGINT path above). SESSION_EXPIRED /
         * KEEPALIVE_FAILED / PROTOCOL_ERROR are RTSP-shaped variants
         * from the sibling `tst_rtp_demux_receiver_end_reason` API that
         * this SRT path never produces — the enum is shared, not the
         * behavior.
         */
        fprintf(stderr, "end reason: %s\n", end_reason_name(reason));
    } else {
        fprintf(stderr, "end_reason query failed (rc=%d): %s\n", er_rc, tst_get_last_error_str());
    }

    tst_managed_demux_receiver_close(rx);
    fprintf(stderr, "done\n");
    return exit_code;
}
