# tst-c examples (C)

Runnable C examples for the `tst-c` C ABI. Linux x86_64 only by build
convention (cbindgen-generated header + cdylib / staticlib).

If you're new to the project and want to start in Rust, the equivalent
curriculum lives at [`../../../examples/`](../../../examples/) — the C
examples here mirror that taxonomy.

## Build

```sh
cd bindings/c
# Transports are opt-in. The offline mux/demux examples build with a bare
# `cargo build`; the transport examples (sending/, receiving/, the rtp/srt
# scenarios) need their feature: e.g. --features srt,rtp (or udp/tcp/hls/rist).
cargo build --features srt,rtp   # produces target/debug/libtstrans.{so,a} + include/tstrans.h
gcc -I include -L ../../target/debug \
    -Wall -Werror -o /tmp/<name> \
    examples/<category>/<name>.c -ltstrans
LD_LIBRARY_PATH=../../target/debug /tmp/<name>
```

For pkg-config-using build systems, `tstrans.pc` is emitted alongside
the cdylib once `tstrans.pc.in` has been substituted by the build —
add `target/debug` to `PKG_CONFIG_PATH` and use
`pkg-config --cflags --libs tstrans`.

## Read in this order

### 1. `getting-started/hello_world.c` — smallest possible mux + KLV

The C twin of the Rust [`hello_world.rs`](../../../examples/getting-started/hello_world.rs).
Builds 1 MPEG-TS frame containing 1 video AU + 1 KLV record using the
`tst_muxer_t` muxer-only handle, drains the bytes, prints the count
(byte-identical to the Rust version: 752 bytes / 4 packets).

```sh
LD_LIBRARY_PATH=../../target/debug /tmp/hello_world
```

### 2. `getting-started/version_check.c` — verify loaded library matches header

Cross-validates every `(runtime accessor, header macro)` pair at process
startup. Queries `tst_get_version_major/minor/patch/packed/string` and
`tst_get_abi_version_major/minor`, compares each against the corresponding
`TST_*_VERSION_*` compile-time macro from `<tstrans.h>`, and exits 0 only
when all values agree.

The canonical pattern for binding authors to copy into their own startup
checks (e.g. `JNI_OnLoad` for `tst-jni`, the UniFFI init hook for
`tst-uniffi`):

```c
if (tst_get_abi_version_major() != TST_ABI_VERSION_MAJOR) {
    fprintf(stderr, "tstrans ABI major mismatch\n");
    exit(1);
}
if (tst_get_abi_version_minor() < TST_ABI_VERSION_MINOR) {
    fprintf(stderr, "tstrans ABI minor too old\n");
    exit(1);
}
```

Expected output (versions will match your build):

```text
Package version (matches Cargo.toml):
  TST_VERSION_MAJOR          runtime=0  header=0  [OK]
  TST_VERSION_MINOR          runtime=1  header=1  [OK]
  TST_VERSION_PATCH          runtime=0  header=0  [OK]
  packed (M<<16|m<<8|p)      runtime=256  header=256  [OK]
  version string             runtime="0.1.0"

ABI contract version (breaking-change cadence):
  TST_ABI_VERSION_MAJOR      runtime=0  header=0  [OK]
  TST_ABI_VERSION_MINOR      runtime=1  header=1  [OK]

After tst_clear_last_error():
  tst_get_last_error()     = 0  (expect 0 = TST_E_SUCCESS)
  tst_get_last_error_str() = ""  (expect empty)

OK: all runtime/header pairs match. Loaded libtstrans is consistent with the tstrans.h compiled into this binary.
```

```sh
LD_LIBRARY_PATH=../../target/debug /tmp/version_check
```

### 3. `muxing/mux_synthetic_srt.c` — sender + synthetic frames over SRT

Open a `tst_mux_sender_t` against an SRT URL, push 5 synthetic H.264 +
KLV frames, close. The C analogue of the Rust
[`send_pipeline_to_socket.rs`](../../../examples/sending/send_pipeline_to_socket.rs).

```sh
LD_LIBRARY_PATH=../../target/debug /tmp/mux_synthetic_srt 127.0.0.1:9000
```

### 4. `muxing/mux_dual_camera.c` — multi-stream within one program

Diff from §3: two video streams (EO + IR) sharing one program + KLV.
Demonstrates `tst_mux_config_add_video_stream` returning per-stream
handles and `tst_muxer_push_video_to(handle, ...)` for fan-out.
Mirrors the Rust [`mux_dual_camera.rs`](../../../examples/muxing/mux_dual_camera.rs).

### 5. `muxing/mux_two_programs.c` — multi-program

Diff from §4: two PMTs in one PAT, each with its own video + KLV
streams. Shows the `tst_program_handle_t` flow and the
`tst_*_to(prog_handle, ...)` siblings. No Rust twin yet — the equivalent
recipe lives in the cookbook ([Repack two single-program inputs into one multi-program TS](../../../docs/cookbook/muxing/repack-multi-program.md))
under a different shape (demux + re-mux instead of synthetic frames).

### 6. `operations/poll_socket_stats.c` — live libsrt wire-stats polling

Push 5 seconds of synthetic video through a `tst_mux_sender_t` and
print RTT + bandwidth + loss + retransmits every 500 ms using
`tst_mux_sender_get_socket_stats`. Shows the operational-telemetry
counterpart to the app-level `tst_mux_sender_get_stats` — wire-level
visibility into what the network actually did (vs what you asked
libsrt to do). The C-side analogue of the
[`operations/`](../../../examples/operations/) Rust examples.

```sh
LD_LIBRARY_PATH=../../target/debug /tmp/poll_socket_stats srt://127.0.0.1:9000
```

### 7. `sending/send_rtp.c` — RTP unicast/multicast sender (raw TS bytes)

Open a `tst_rtp_sender_t` to an `rtp://` URL, push 100 synthetic 188-byte
MPEG-TS null packets via `tst_rtp_sender_send_ts`, then close. The
lowest-level RTP send API — caller supplies pre-built TS packets; the
library handles RTP framing (RFC 2250), UDP packetisation, and SSRC/sequence
management. Use `tst_rtp_mux_sender_t` instead when you need the library to
also mux encoded video/KLV/audio into TS for you.

```sh
# unicast (loopback default)
LD_LIBRARY_PATH=../../target/debug /tmp/send_rtp

# explicit unicast destination
LD_LIBRARY_PATH=../../target/debug /tmp/send_rtp --dest 192.168.1.100:5000

# multicast (org-local scope, RFC 2365)
LD_LIBRARY_PATH=../../target/debug /tmp/send_rtp --dest 239.1.2.3:5000
```

### 8. `sending/send_rtsp_server.c` — RTSP server: unicast mount + push loop

Start an RTSP server, register a `/live` mount, and push synthetic H.264 +
MISB ST 0601 KLV frames in a 30 fps loop. Multiple RTSP clients can connect
simultaneously — each gets its own view of the same fanout channel.

Key concepts demonstrated:
- Builder chain: `_max_sessions`, `_session_timeout`, `_fanout_capacity`,
  `_graceful_shutdown_drain_ms`.
- Unicast mount registration (`tst_rtsp_server_add_unicast_mount`) and how
  it differs from the SRT point-to-point sender.
- Signal-safe shutdown: `tst_rtsp_server_cancel_handle` + SIGINT handler,
  followed by `tst_rtsp_server_stop` (graceful drain + RFC 7826 Notice 5402)
  from the main thread.
- Per-mount stats polling (`tst_rtsp_mount_get_stats`) and server stats
  (`tst_rtsp_server_get_stats`).

```sh
# Terminal 1 — start the server
LD_LIBRARY_PATH=../../target/debug /tmp/send_rtsp_server \
    --bind rtsp://0.0.0.0:8554 --mount /live
# Terminal 2 — play the stream
ffplay rtsp://127.0.0.1:8554/live
```

## Receiver-side C examples

The C ABI covers both sender (mux + raw/TS sender) and receiver (demux +
raw/TS receiver) surfaces. Receiver-side C examples ship under
[`receiving/`](receiving/):

- [`receiving/recv_ts_to_file.c`](receiving/recv_ts_to_file.c) — TS-bytes
  receiver: drain TS packets straight to a file via the C ABI's Receiver
  shape.
- [`receiving/recv_demux_to_console.c`](receiving/recv_demux_to_console.c)
  — DemuxReceiver event walk: print one line per demux event to stdout.
- [`receiving/recv_klv_to_stdout.c`](receiving/recv_klv_to_stdout.c) — KLV
  extraction: dump KLV payloads as hex / file on the side of a Demuxer.
- [`receiving/recv_raw_to_file.c`](receiving/recv_raw_to_file.c) — raw
  socket bytes (no TS framing): the lowest-level RawReceiver shape.
- [`receiving/recv_rtp.c`](receiving/recv_rtp.c) — RTP
  multicast demux receiver: join an IP multicast group, walk the full
  `tst_event_t` event-kind switch, and shut down gracefully on SIGINT via
  `tst_rtp_demux_receiver_cancel`. The RTP twin of
  `recv_demux_to_console.c`. Requires `TST_HAS_RTP`.
- [`receiving/recv_rtsp_camera.c`](receiving/recv_rtsp_camera.c) —
  full RTSP client lifecycle: builder chain with Digest MD5 auth and
  transport preference (UDP / TCP-interleaved / auto), `_connect`,
  `_play`, `into_demux_receiver` bridge to typed event loop, SIGINT
  cancel, and cleanup. Canonical pattern for consuming a gimbaled-platform
  camera stream over RTSP.
- [`receiving/recv_srt_events.c`](receiving/recv_srt_events.c) — the
  MANAGED (auto-reconnecting) SRT demux receiver reference example: full
  `tst_event_t` kind coverage including `TST_EVENT_KIND_RECONNECT_DISCONTINUITY`
  (the boundary marker `recv_demux_to_console.c`'s switch has no case
  for), inline MISB ST 0601 KLV decode via `tst_st0601_decode` /
  `tst_st0601_geometry`, and the documented cancel-then-close SIGINT
  shutdown ordering. Supersedes `recv_demux_to_console.c` for the
  managed+caller+KLV-decode case, and is the behavioral reference the
  Apple/Swift wrapper is written against.
