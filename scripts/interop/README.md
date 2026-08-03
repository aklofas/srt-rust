# Interop evidence: transport-axis matrix

`run-matrix.sh` exchanges the `tst-interop` driver's synthetic MPEG-TS/KLV
traffic with real third-party tools (ffmpeg, TSDuck's `tsp`, GStreamer, VLC)
over live network sessions on this box, across every transport this crate
supports (SRT, RIST, UDP, TCP, HLS, RTSP), and writes one evidence JSON file
per "cell" (one peer, one direction, one transport, one optional variant like
encryption).

This is the arc's core deliverable: real tools talking to this codebase over
a real wire, not another closed-loop self-test.

**linux-x86_64 only.** Peer tools are discovered at runtime (`have()` in
`lib.sh`) — a missing tool produces a `SKIPPED_TOOL_MISSING` cell, never a
fake pass or fail. Requires `jq` and `python3` on `PATH` in addition to
whichever peer tools you want exercised.

## Running it

```bash
# Everything, default 10s per cell:
bash scripts/interop/run-matrix.sh --outdir /tmp/interop-run

# Just the SRT block, while iterating on it:
bash scripts/interop/run-matrix.sh --outdir /tmp/interop-run --cells 'srt/*'

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
`expectations.toml`. That file ships with zero real rows — until Task 12
populates it, running the full matrix will (correctly) exit nonzero on every
genuine gap it finds. That is the intended behavior, not a bug in this
script: see `report.rs`'s module doc for why an unmatched `FAIL` must never
be silently absorbed.

## Cell id / tier / direction conventions

- **Cell id**: `<transport>/<direction>-<peer>[-encrypted]`, e.g.
  `srt/us-to-ffmpeg`, `srt/tsp-to-us-encrypted`, `rtsp-serve/vlc-probe`.
  `report merge`'s expectations grammar treats the id's segment before the
  first `/` as the axis (`report render`'s per-axis table grouping uses the
  same split) and supports a trailing `*` glob, e.g. `srt/*` matches every
  SRT cell.
- **`direction`** in the emitted cell JSON always names *our* role:
  `send` = `tst-interop` pushes or serves; `recv` = `tst-interop` listens
  and receives. `n/a` for the one cell where `tst-interop` only brackets a
  peer-to-peer exchange (see `rtsp-consume` below).
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
  - `n/a` — decode-only probes (`rtsp-serve/vlc-probe`) with no capture
    file to compare against anything; PASS means "no error/fatal marker in
    the peer's own log," mirroring the outer project's
    `release-validation.sh` decoder-compatibility step.

## Known, already-evidenced gaps (read before re-chasing these)

A full local run (`--seconds 8`, every peer tool installed) is stable at
**8 PASS / 17 FAIL / 0 SKIPPED**, reproduced identically across independent
runs. Every `FAIL` is a genuine, understood finding from developing and
running this script — not a bug in the orchestrator — falling into five
root causes below. This is exactly what Task 12's `expectations.toml` rows
exist to document with a reason + reference, not something Task 11 should
paper over.

1. **ffmpeg `-c copy` drops PTS on our KLV PID (10 of 17 FAILs).** Every
   `ffmpeg`-remux cell where ffmpeg reads our KLV private-data stream (PMT
   stream_type `0x06`, registration descriptor `"KLVA"`) and re-muxes it
   with `-c copy -copy_unknown` writes the KLV PES packets with *no*
   PTS/DTS field at all (`tsp -P pes` on the remuxed file shows a 9-byte
   PES header — no timestamp fields present; ffmpeg itself warns
   "Timestamps are unset in a packet for stream 1"). This reproduces
   identically on a pure file-to-file remux with no network transport
   involved at all — `-map 0` is required (ffmpeg's default automatic
   stream selection excludes data/unknown-codec streams, so without it the
   KLV PID is silently absent from the output rather than present-but-
   timestamp-less — the exact opposite failure mode, so don't drop it when
   reproducing this):

   ```
   tst-interop gen --profile baseline --seconds 5 --out in.ts
   ffmpeg -y -i in.ts -map 0 -c copy -copy_unknown -f mpegts out.ts
   tst-interop verify --file out.ts --expect baseline --seconds 5
   ```

   Re-run to confirm this citation before relying on it: ffmpeg's own
   stderr includes `Timestamps are unset in a packet for stream 1. This is
   deprecated and will stop working in the future.`, and `tst-interop
   verify` reports `verify: FAIL (baseline): KLV records: got 0, want >= 35
   (10 Hz x 5s x 70% slack)` — the identical symptom every live-transport
   ffmpeg-remux cell in this matrix shows. It is an ffmpeg mpegts-demux/mux
   limitation with unknown/data elementary streams, not a transport-
   specific bug. `-copy_unknown` and `-fflags +genpts` were both tried and
   neither restores a KLV record count in the remuxed capture. Affects:
   `hls/ffmpeg-pull`,
   `rist/ffmpeg-to-us`, `srt/{us,ffmpeg}-to-{ffmpeg,us}[-encrypted]` (4
   cells), `tcp/us-to-ffmpeg`, `tcp/ffmpeg-to-us`, `udp/ffmpeg-to-us`,
   `rtsp-serve/ffmpeg-pull` — every cell where ffmpeg touches our KLV PID
   with `-c copy`, full stop.

2. **SRT (only) loses a small tail even with a paced/lingered `tsp`/`gst`
   sender (3 of 17 FAILs).** `tsp -P regulate -O srt --caller ... --linger
   5` (needed — without `-P regulate`, `tsp` bursts an entire file's worth
   of packets almost instantly and the live transport drops nearly all of
   it; see the `-P regulate` note below) and a similarly-paced `gst filesrc
   ! tsparse set-timestamps=true ! srtsink` both land comfortably inside
   `tst-interop`'s 70% nominal-count slack (`recv: PASS`), but the received
   `stream_sha256` does not exactly match the source file's — a handful of
   trailing video AUs/KLV records (order of magnitude 3% for `tsp`, ~18%
   for `gst`, in local testing) never arrive. **This is SRT-specific, not
   a generic "any live recv loses a tail" effect**: the identical
   `tsp -P regulate` pattern over RIST (`rist/tsp-to-us`) and UDP
   (`udp/tsp-to-us`) is **byte-perfect** (PASS) in the same run — only the
   SRT direction shows the mismatch. Plausible cause not yet root-caused:
   a timing gap between `tsp`'s regulated-pacing completion + 5s linger and
   our own recv-side deadline (`seconds` + a 2s post-start grace,
   `crates/tst-interop/src/recv.rs`'s `POST_START_GRACE`) closing the
   session slightly early — worth a closer look with `--statistics-interval`
   on the `tsp` side before writing this up as an expectation in Task 12.
   The `transparent`-tier byte-hash comparison this script does for
   `tsp`/`gst` peer-to-us SRT cells is specifically designed to surface
   this class of gap (see the Task 11 dispatch's ledger note about a
   related, already-tracked "live recv loses the final video AU at
   teardown" finding), not something to loosen away. Affects:
   `srt/tsp-to-us[-encrypted]`, `srt/gst-to-us`.

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
   time than a direct SRT/TCP connect, worth revisiting with a longer
   `--seconds` for RTSP cells specifically if Task 12 wants tighter margin
   instead of an expectation row.

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
  however, fix the PTS-loss finding above.
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

Every cell in this script is built from one of four shared shapes in
`run-matrix.sh` (`run_send_peer_recv` / `run_peer_send_recv` /
`run_serve_peer_pull` / `run_serve_peer_probe`) — read their doc comments
first; a new transport×peer×direction combination is almost always a
one-line call to an existing shape, not new plumbing. `lib.sh` holds the
shape-independent primitives (`have`, `free_port`, `cell_timeout`,
`emit_pass`/`emit_fail`/`emit_skipped`, `metrics_only`).

**`metrics_only` is load-bearing** — `tst-interop recv`/`verify --json`
both write a `VerifyReport` (`{pass, failures, metrics: {...}}`), one level
of nesting deeper than the bare `CellMetrics` object `send --json` writes
and than what a `RawCell.metrics` field expects (see `report.rs`). Every
call site that has a `recv`/`verify` JSON file must route it through
`metrics_only` before handing it to `emit_pass`/`emit_fail` — passing the
`VerifyReport` file straight through embeds the wrong shape and `report
merge` fails to parse the cell entirely (missing `video_aus` etc.).
