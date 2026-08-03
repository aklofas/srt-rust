# Interop evidence: transport + format matrix

`run-matrix.sh` exchanges the `tst-interop` driver's synthetic MPEG-TS/KLV
traffic with real third-party tools (ffmpeg, TSDuck's `tsp`, GStreamer, VLC,
mpv) over live network sessions AND local per-profile analyzer/decode
probes on this box, across every transport this crate supports (SRT, RIST,
UDP, TCP, HLS, RTSP) and every one of the 12 canonical stream profiles
(`crates/tst-interop/src/profiles.rs`), and writes one evidence JSON file
per "cell" (one peer, one direction, one transport-or-local-probe, one
optional variant like encryption or profile).

This is the arc's core deliverable: real tools talking to this codebase over
a real wire (or a real local decode/analysis pass), not another closed-loop
self-test.

**linux-x86_64 only.** Peer tools are discovered at runtime (`have()` in
`lib.sh`) — a missing tool produces a `SKIPPED_TOOL_MISSING` cell, never a
fake pass or fail. Requires `jq` and `python3` on `PATH` in addition to
whichever peer tools you want exercised.

## Running it

```bash
# Everything (all 12 profiles, transport axis pinned to baseline —
# see --profiles's own --help text), default 10s per cell:
bash scripts/interop/run-matrix.sh --outdir /tmp/interop-run

# Just the SRT transport block, while iterating on it:
bash scripts/interop/run-matrix.sh --outdir /tmp/interop-run --cells 'srt/*'

# Just the format axis, a couple of profiles, for a fast local loop:
bash scripts/interop/run-matrix.sh --outdir /tmp/interop-run \
  --cells 'analyze/*' --profiles baseline,h266-klv

# Shorter cells for a fast local loop, longer for more margin on a loaded box:
bash scripts/interop/run-matrix.sh --outdir /tmp/interop-run --seconds 5
```

Output layout under `--outdir`:

```
DIR/
  meta.json        # host, date, seconds-per-cell, tool versions
  cells/*.json      # one RawCell JSON per cell (see report.rs's doc comment)
  logs/*.log        # combined (our side + peer side) log per cell
  work/*             # generated source .ts files, per-cell captures, intermediate metrics
  results.json      # `report merge`'s output (expectations-applied)
  results.md        # `report render`'s markdown table, same shape as the published evidence page
```

Exit code is `report merge`'s: 0 iff every `FAIL` matched a row in
`expectations.toml`. That file now carries a real row for every genuine gap
this matrix has surfaced (73 as of task 12, see "Known, already-evidenced
gaps" below) — running the full matrix exits 0. Any *new* `FAIL` an
expectations row doesn't already cover still exits nonzero: see
`report.rs`'s module doc for why an unmatched `FAIL` must never be silently
absorbed.

## Cell id / tier / direction conventions

- **Cell id**: transport-axis cells are `<transport>/<direction>-<peer>[-encrypted]`,
  e.g. `srt/us-to-ffmpeg`, `srt/tsp-to-us-encrypted`, `rtsp-serve/vlc-probe`.
  Format-axis cells (task 12) add a third segment, the profile name:
  `<axis>/<peer>/<profile>`, e.g. `decode/mpv/h266-klv`,
  `srt-live/tsp-to-us/baseline`. `report merge`'s expectations grammar
  treats the id's segment before the *first* `/` as the axis (`report
  render`'s per-axis table grouping uses the same split) and supports a
  trailing `*` glob, e.g. `srt/*` matches every SRT transport-axis cell,
  `decode/*` matches every decode format-axis cell across every profile,
  `decode/mpv/*` matches every mpv decode cell across every profile.
- **`direction`** in the emitted cell JSON always names *our* role:
  `send` = `tst-interop` pushes or serves; `recv` = `tst-interop` listens
  and receives; `n/a` for cells with no us-side transport role at all —
  the one transport-axis cell where `tst-interop` only brackets a
  peer-to-peer exchange (see `rtsp-consume` below), and every format-axis
  `analyze/*`/`decode/*` cell (local probes against an already-generated
  file, no transport leg on any side).
- **`peer`**: the third-party tool's binary name (`ffmpeg`, `tsp`,
  `gst-launch-1.0`, `cvlc`), or `vlc+ffmpeg` for the one two-peer cell.
- **`tier`**: what the cell asserts, beyond `tst-interop verify`/`recv`
  reporting `pass`:
  - `transparent` — the capture must be **byte-for-byte identical** to
    what was sent (`stream_sha256` equality). Used for `tsp` (a pure
    relay/dump tool with no re-encoding path) and `gst` (`srtsrc`/
    `srtsink` are raw byte pass-throughs in this pipeline shape).
  - `remux` — the capture only has to satisfy `tst-interop verify`'s
    profile invariants (video AU/KLV-record counts within the documented
    70% slack, correct codec/carriage, monotonic PTS, etc.) — used for
    `ffmpeg`/`gst`-decode/HLS/RTSP cells, where the peer actively
    re-packetizes (HLS segmenting, RTSP interleaving) or is known to touch
    PES framing (see the KLV-PTS finding below).
  - `n/a` — decode-only probes (`rtsp-serve/vlc-probe`, every format-axis
    `decode/*` cell) with no capture file to compare against anything;
    PASS means "no error/fatal marker in the peer's own log" (plus mpv's
    own explicit no-stream-selected check — see lib.sh), mirroring the
    outer project's `release-validation.sh` decoder-compatibility step.
    Also used for every format-axis `analyze/*` cell (a structural/
    counter assertion, not a byte- or profile-invariant comparison).

## Known, already-evidenced gaps (read before re-chasing these)

**Transport axis** (the 25 `srt`/`udp`/`rist`/`tcp`/`hls`/`rtsp-*` cells,
run against the `baseline` profile only): a full local run (`--seconds 8`,
every peer tool installed) is stable at **8 PASS / 17 FAIL / 0 SKIPPED**,
reproduced identically across independent runs (including a re-run in Task
12, after the `tst-pipeline` "flush pending PES on terminal receive
errors" library fix landed upstream — zero change to any transport-axis
verdict, confirming that fix's scope genuinely didn't overlap these
findings). Every `FAIL` is a genuine, understood finding from developing
and running this script — not a bug in the orchestrator — falling into
five root causes below. This is exactly what `expectations.toml`'s rows
exist to document with a reason + reference, not something to paper over.

1. **ffmpeg `-c copy` corrupts our KLV PID's elementary-stream payload
   (10 of 17 transport-axis FAILs, +2 more on the `srt-live/*` and
   `decode`-adjacent format-axis cells Task 12 added — see below).**
   ★Task 12 refined this finding: it is **not** primarily a PTS-loss bug
   as originally described — it is a **5-byte payload truncation** ffmpeg
   applies to every KLV PES packet it demuxes, independent of whether a
   PTS was ever present. Verified byte-for-byte with `tsp -P pes
   --save-es` (extracts the raw elementary stream) on a pure file-to-file
   remux with no network transport involved at all:

   ```
   tst-interop gen --profile baseline --seconds 5 --out in.ts
   ffmpeg -y -i in.ts -map 0 -c copy -copy_unknown -f mpegts out.ts
   tst-interop verify --file out.ts --expect baseline --seconds 5
   # -map 0 is required — ffmpeg's default stream selection excludes
   # data/unknown-codec streams, so without it the KLV PID is silently
   # ABSENT from the output rather than present-but-corrupted, the
   # opposite failure mode, so don't drop it when reproducing this.
   ```

   `tst-interop verify` reports `KLV records: got 0, want >= 35` — the
   identical symptom every live-transport ffmpeg-remux cell in this
   matrix shows. **The PMT itself survives the remux intact**
   (`tspsi in.ts` vs `tspsi out.ts`: both show `Elementary stream: type
   0x06 (MPEG-2 PES private data)` + the `Registration (0x05)` descriptor
   with format identifier `"KLVA"`, only the PID number changes) — so
   this is not a PMT/stream-classification loss, and OUR demuxer's
   classification requirement (stream_type 0x06 + `KLVA` registration
   descriptor) is unaffected. `tsp -P pes --pid <klv> --header` on both
   files shows a spec-compliant, PTS-less 9-byte optional header in
   *both* `in.ts` and `out.ts` (`baseline`'s async/`PrivateData` KLV
   carriage is `carries_pts: false` by design —
   `crates/tst-interop/src/mux_setup.rs`'s `build_config` — so the
   original never had PTS to lose in the first place; confirmed
   independently via `tsanalyze --normalized`'s `pts=0` count on the KLV
   PID in both files, and via `pes=50`/`pes=50` — same PES packet count,
   ruling out packet drops). The actual damage is in the **payload**:
   `tsp -P pes --pid <klv> --save-es` on `in.ts` shows every KLV record
   starting with the 16-byte SMPTE Universal Label
   `06 0E 2B 34 02 0B 01 01 0E 01 03 01 01 00 00 00...` (the ST 0601
   local-set key); the same extraction on `out.ts` shows
   `0B 01 01 0E 01 03 01 01 00 00 00...` — **the leading 5 bytes
   (`06 0E 2B 34 02`) of every single KLV record are gone**, corrupting
   the UL key our demuxer requires to recognize a record at all, hence
   "KLV records: got 0". The missing-byte-count (5) exactly matches what
   a PTS-only PES optional header would occupy, which — combined with the
   companion `klv-sync` finding directly below — strongly suggests
   ffmpeg's mpegts demuxer applies an unconditional 5-byte consumption to
   any elementary stream it classifies as `klv` via the `KLVA`
   registration descriptor, regardless of what the packet's own
   `PES_header_data_length` says.

   **The `klv-sync` profile (`SynchronousMetadata`, stream_type `0x15`,
   which *does* carry a real H.222.0 §2.12.4.2 5-byte `Metadata_AU_cell`
   header + a real PES PTS) shows a *different*, and more revealing,
   symptom** — not "KLV records: got 0" but
   `verify: FAIL (klv-sync): KLV carriage: expected Sync, observed
   {Async}`. Byte-level inspection explains it: `tspsi out.ts`'s PMT now
   reports the KLV PID as **stream_type `0x06` (PrivateData)** — ffmpeg's
   *own output muxer* downgrades `0x15` to `0x06` on remux, even though
   it correctly read `0x15` + the `KLVA` descriptor on input (its own
   stderr identifies the stream as `Data: klv (KLVA / 0x41564C4B)`). And
   `tsp -P pes --save-es` on the remuxed KLV PID shows the SAME 5-byte
   strip — but this time those 5 bytes *were* the genuine
   `Metadata_AU_cell` header (`in.ts`'s payload starts
   `00 00 df 00 32 06 0e 2b 34...` — AU-cell header, then the UL key;
   `out.ts`'s starts directly at `06 0e 2b 34...`). Put together: ffmpeg's
   demux side evidently assumes every `klv`-classified elementary stream
   is AU-cell-wrapped (true for `SynchronousMetadata`, which is the only
   carriage its own muxer/demuxer round-trips a KLV codec_id through) and
   unconditionally strips what it treats as that 5-byte header — correct
   for `klv-sync` (which really is AU-cell-wrapped, hence the stream_type
   downgrade to `0x06`+no-wrap on write-back, since ffmpeg apparently has
   no output path that re-wraps+re-classifies as `0x15`) but destructive
   for `PrivateData`/async KLV (this codebase's convention: raw,
   *unwrapped* KLV bytes on `0x06` — see
   `reference_klv_au_cell_caller_responsibility` — a valid MISB/ST 1402
   carriage ffmpeg's `klv` codec apparently doesn't account for).

   This is an ffmpeg-side mpegts demux/mux limitation with the `klv`
   codec_id, not a bug in this codebase's own PMT signaling or PES
   framing (both independently confirmed spec-compliant, byte-for-byte,
   on both KLV carriage modes) — **not a "our demuxer could legitimately
   accept this" case**: the bytes ffmpeg delivers for async/`PrivateData`
   KLV are genuinely truncated/corrupted (a valid UL key becomes an
   invalid one), so there is nothing for a more-lenient demuxer to
   legitimately accept. `-copy_unknown` and `-fflags +genpts` were both
   tried and neither restores a KLV record count. Affects every
   ffmpeg-remux cell that touches this codebase's KLV PID with `-c copy`,
   full stop — transport-axis: `hls/ffmpeg-pull`, `rist/ffmpeg-to-us`,
   `srt/{us,ffmpeg}-to-{ffmpeg,us}[-encrypted]` (4 cells),
   `tcp/us-to-ffmpeg`, `tcp/ffmpeg-to-us`, `udp/ffmpeg-to-us`,
   `rtsp-serve/ffmpeg-pull`; format-axis (`srt-live/*`, one profile
   each): `srt-live/us-to-ffmpeg` and `srt-live/ffmpeg-to-us` for every
   non-`klv-sync` profile (same UL-key-truncation symptom, "KLV records:
   got 0") and for `klv-sync` specifically (the carriage-downgrade
   symptom above).

2. **SRT (only) loses a small tail even with a paced/lingered `tsp`/`gst`
   sender (3 of 17 transport-axis FAILs; reproduces on every profile's
   `srt-live/tsp-to-us` cell too).** `tsp -P regulate -O srt --caller ...
   --linger 5` (needed — without `-P regulate`, `tsp` bursts an entire
   file's worth of packets almost instantly and the live transport drops
   nearly all of it; see the `-P regulate` note below) and a
   similarly-paced `gst filesrc ! tsparse set-timestamps=true ! srtsink`
   both land comfortably inside `tst-interop`'s 70% nominal-count slack
   (`recv: PASS`), but the received `stream_sha256` does not exactly
   match the source file's — a handful of trailing video AUs/KLV records
   (order of magnitude 3% for `tsp`, ~18% for `gst`, in local testing)
   never arrive. **This is SRT-specific, not a generic "any live recv
   loses a tail" effect**: the identical `tsp -P regulate` pattern over
   RIST (`rist/tsp-to-us`) and UDP (`udp/tsp-to-us`) is **byte-perfect**
   (PASS) in the same run — only the SRT direction shows the mismatch.

   ★Task 12 root-caused this (bounded investigation, live SRT session
   with wall-clock instrumentation on both sides): the loss is **not** a
   sender-side drain/linger problem. `tst-interop recv`'s `Teeing` tap
   (`crates/tst-interop/src/transport.rs`) tallies bytes at the transport
   boundary — *below* the demuxer, i.e. exactly what libsrt delivered to
   this codebase's own code — and it already shows the shortfall
   (87044/90240 bytes on an 8s baseline run), which rules out "we
   received it but discarded it during our own processing" and confirms
   libsrt itself never handed those bytes to the recv side. Tripling
   `tsp`'s `--linger` (5s → 15s, comfortably longer than
   `tst-interop recv`'s own deadline) reproduced the **exact same byte
   count** — ruling out "the sender didn't wait long enough to drain" as
   the cause. By elimination — and confirmed by direct wall-clock
   timestamps: `recv`'s own process exit landed within 3ms of `tsp`'s,
   regardless of `tsp`'s linger length — **the cause is
   `tst-interop recv`'s own receive-side deadline**
   (`crates/tst-interop/src/recv.rs`: anchored once, at the *first*
   received event, to `seconds + POST_START_GRACE` where
   `POST_START_GRACE` is a flat 2 seconds) **closing the connection
   (`rx.close()`) while `tsp`'s PCR-paced `-P regulate` sender is still
   transmitting the tail of the file** — `-P regulate`'s real-world
   pacing (plus process/scheduling overhead) evidently runs slightly
   longer, wall-clock, than the nominal `seconds` duration, and SRT's
   connection handshake delays exactly when the "first event" anchor
   fires relative to when `tsp`'s own pacing clock started at file-open —
   eating directly into the fixed 2-second grace margin in a way UDP/RIST
   (connectionless, first-byte arrives essentially at send-start) don't
   experience. This is a plausible, mechanistically-consistent
   explanation for the SRT-vs-RIST/UDP asymmetry (not independently
   re-verified against a live RIST timing capture in this session — the
   deadline-vs-linger elimination above is the directly-verified part).
   **Library-side follow-up candidate** (out of scope for this
   scripts-only task — noted here, not filed as a formal backlog entry):
   either `POST_START_GRACE` needs headroom for connection-establishment
   latency specifically, or `recv`'s deadline should tolerate an explicit
   "sender still draining" signal rather than a flat post-first-byte
   window. The `transparent`-tier byte-hash
   comparison this script does for `tsp`/`gst` peer-to-us SRT cells is
   specifically designed to surface this class of gap, not something to
   loosen away. Affects: `srt/tsp-to-us[-encrypted]`, `srt/gst-to-us`,
   and `srt-live/tsp-to-us` for every profile.

3. **`ffmpeg` hangs against a live RIST or UDP listener with nothing ever
   received (2 of 17 FAILs).** `rist/us-to-ffmpeg`: ffmpeg's librist
   listener registers a peer/flow ("Listening peer 2 timed out after
   ~300ms", "Flow ... is dead") but the output file stays empty; ffmpeg has
   to be force-killed (`timeout --kill-after`, exit 137). `udp/us-to-ffmpeg`:
   ffmpeg successfully *probes* the stream (both streams correctly
   identified, output mapping set up, "Press [q] to stop" printed) but then
   never writes a single byte to the output file before being force-killed
   — reproduces identically with `-fflags nobuffer` and with longer
   pre-send settle times, and reproduces with `-loglevel info`/`warning`
   equally, so it isn't a startup-race artifact of this script's own
   timing. Both are peer-side (ffmpeg/librist-input-path) issues, not
   `tst-interop` send-side ones — the identical `send` calls work
   correctly against `tsp` listeners on the same two transports in the
   same run (`rist/us-to-tsp`, `udp/us-to-tsp` both PASS byte-perfect).

4. **`rtsp-consume/vlc-serve-ffmpeg-pull` (1 of 17 FAILs, best-effort by
   design).** VLC's `--sout '#rtp{sdp=rtsp://:PORT/s}'` RTSP serving
   returned a "5XX Server Error" to ffmpeg's DESCRIBE in local testing.
   Wired as `known_flaky`-bound from day one per the plan — see the
   "RTSP-consume has no `tst-interop` transport leg at all" note below for
   why this cell can't exercise this crate's own code either way.

5. **`rtsp-serve/vlc-probe`: "main decoder error: buffer deadlock
   prevented" (1 of 17 FAILs).** Plausibly this crate's own synthetic
   fixture, not the wire protocol: `crates/tst-interop/src/fixtures.rs`'s
   H.264 generator only builds a real, decodable SPS/PPS/IDR on keyframes
   (every 30th frame) — every inter-frame AU is `0xA5`-filler bytes wrapped
   in a bare NAL header, which a real decoder (VLC here; ffmpeg logs the
   same "decode_slice_header error"/"no frame!" pattern on every ffmpeg
   cell too, just without failing the *cell* since `-c copy` doesn't need a
   successful decode) cannot decode. `rtsp-serve/ffmpeg-pull`'s own
   `verify` FAIL additionally shows a real video-AU shortfall (149/240,
   just under the 70% floor) beyond the universal KLV issue above — RTSP's
   TCP-interleaved SETUP/PLAY handshake plausibly costs a bit more startup
   time than a direct SRT/TCP connect. Left as an expectation row rather
   than special-cased with a longer `--seconds` for RTSP specifically —
   that would diverge this one cell's timing from every other cell's
   shared `--seconds` budget for a margin call this close to the 70%
   floor, not a clear win.

## Peer command-line notes (deviations from the plan's starting sketches)

- **`tsp -I file ... -O <srt|rist|ip> ...` needs `-P regulate` inserted**
  between the file input and the live-network output plugin. Without it,
  `tsp` reads the whole file and pushes it essentially as fast as
  `srt_send()`/librist will accept, finishing in tens of milliseconds
  regardless of the file's nominal duration — the live transport's
  congestion control/flow window can't absorb a burst that size, and
  almost everything gets dropped (confirmed: without `-P regulate`, a
  56 KB / 5 s file arrived as ~5 KB / 4 SRT packets before `tsp` exited
  "successfully"). `-P regulate` paces the packet flow to the file's
  PCR-derived bitrate, matching how a real live sender behaves. The SRT
  side additionally needs `--linger 5` on `tsp -O srt` (SRT's own
  "Default: no linger" — an unlingered close discards whatever's still
  queued in libsrt's send buffer at close time, the same drain-before-close
  concern `crates/tst-interop/src/transport.rs`'s own module doc describes
  for this crate's SRT sender).
- **`ffmpeg -copy_unknown`** is required alongside `-c copy -map 0` for
  ffmpeg to carry our KLV private-data stream through a remux at all (its
  default stream-copy mapping otherwise silently drops it) — it does not,
  however, fix the payload-truncation finding above.
- **`ffmpeg -passphrase` position matters.** It's an SRT-protocol AVOption:
  it must sit immediately before whichever `-i`/output URL is the SRT side.
  For `srt://.../ffmpeg-to-us`, ffmpeg reads a plain local file and writes
  to SRT, so `-passphrase` goes *after* `-i $GEN_FILE`, right before
  `-f mpegts srt://...` — putting it before `-i` (matching the sibling
  `us-to-ffmpeg` cell, where ffmpeg's *input* is the SRT side) fails with
  `Option passphrase not found`.
- **`gst-launch-1.0 filesrc ! srtsink` needs `tsparse set-timestamps=true`**
  in between to get any real-time pacing at all — a bare `filesrc` has no
  timestamps on its raw-byte-stream buffers, so nothing downstream can
  sync to wall-clock time without it. `tsparse` derives per-buffer
  timestamps from the stream's own PCR and smooths them
  (`smoothing-latency=100000`, microseconds).
- **RIST profile defaults line up across every tool for free**: this
  crate's `RistConfig::default()`, `tsp`'s `--profile` ("main profile by
  default"), and ffmpeg's `-rist_profile` (`default main`) all default to
  RIST Main profile — no `?profile=` override needed anywhere in this
  matrix.
- **`rist://@host:port` = bind/listen, `rist://host:port` = connect/send**
  is the shared convention across `tst-rist`, `tsp -I/-O rist`, and
  ffmpeg's `rist://` protocol — the same `@`-prefix idiom `tst-udp`/
  `tst-tcp`/`tst-rist`'s own URL parsers use for "this is a receive-side
  bind" (see e.g. `crates/tst-rist/src/url.rs`'s module doc).
- **RTSP-consume has no `tst-interop` transport leg at all.**
  `crates/tst-interop/src/transport.rs`'s `make_recv` only dispatches
  `udp`/`tcp`/`tcps`/`rist`/`srt` — there is no `rtsp://` connect-side
  support (RTSP only appears as a *serve* scheme, driven by `send`; see
  `serve.rs`'s module doc for why HLS/RTSP work that way). So
  `rtsp-consume/vlc-serve-ffmpeg-pull` is peer-to-peer only: `tst-interop`
  contributes the source file (`gen`) and the final verification
  (`verify`), while VLC serves it over RTSP (`--sout
  '#rtp{sdp=rtsp://:PORT/s}'`) and ffmpeg pulls it. Wired from day one as
  a likely `known_flaky` candidate for Task 12 — VLC's `--sout` RTSP
  serving is fiddly and this cell doesn't exercise this crate's own RTSP
  code at all either way.

## Adding a cell

Transport-axis cells are built from one of four shared shapes in
`run-matrix.sh` (`run_send_peer_recv` / `run_peer_send_recv` /
`run_serve_peer_pull` / `run_serve_peer_probe`) — read their doc comments
first; a new transport×peer×direction combination is almost always a
one-line call to an existing shape, not new plumbing. Format-axis cells
(local, no-transport analyzer/decode probes, and the `srt-live/*`
per-profile SRT block) have their own three shapes just below the
transport-axis ones (`run_analyze_ffprobe` / `run_analyze_tsanalyze` /
`run_analyze_tsp` / `run_decode_probe`, plus `srt_live_cells_for_profile`
reusing `run_send_peer_recv`/`run_peer_send_recv` directly) — a new
profile added to `crates/tst-interop/src/profiles.rs`'s registry needs no
new plumbing here either, just `lib.sh`'s `ALL_PROFILE_NAMES` updated (and
`expected_stream_count` if the new profile's program/audio shape isn't
already covered by that function's formula). `lib.sh` holds the
shape-independent primitives (`have`, `free_port`, `cell_timeout`,
`emit_pass`/`emit_fail`/`emit_skipped`, `metrics_only`, plus the
format-axis's `expected_stream_count`/`tsanalyze_ts_line_counters_zero`/
`tsp_analyze_counters_zero`/`DECODE_PAYLOAD_NOISE` family — see their own
doc comments).

**`metrics_only` is load-bearing** — `tst-interop recv`/`verify --json`
both write a `VerifyReport` (`{pass, failures, metrics: {...}}`), one level
of nesting deeper than the bare `CellMetrics` object `send --json` writes
and than what a `RawCell.metrics` field expects (see `report.rs`). Every
call site that has a `recv`/`verify` JSON file must route it through
`metrics_only` before handing it to `emit_pass`/`emit_fail` — passing the
`VerifyReport` file straight through embeds the wrong shape and `report
merge` fails to parse the cell entirely (missing `video_aus` etc.).
