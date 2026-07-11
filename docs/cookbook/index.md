# Cookbook

> **Who this is for:** You've read the guides and want to crib working code for a specific task. Most recipes are self-contained Rust snippets (or example pointers); a few binding-specific recipes (e.g. Python + PyAV, noted inline) cover tasks that are inherently language-specific. Paste them into your project.

> **You will learn:**
> - Where to find a recipe by topic
> - How recipes group by reader intent (sending, receiving, KLV, codecs, operations)
> - Where to find the full runnable example for each recipe (where one exists)

Recipe numbers are stable across edits — existing inbound links stay valid as recipes are added or rearranged. Within each section, recipes are listed in numeric order, not narrative order. Run any example with `cargo run -p tst-examples --example <name>`.

## Browse by section

### 🛰 Sending — produce a TS stream

- **Recipe 0: Send a single TS packet to any `Transport`** — [00-send-single-packet.md](sending/00-send-single-packet.md) — The simplest possible sender — open a transport, push 188 bytes, drop.
- **Recipe 1: Send video + KLV with passphrase encryption** — [01-send-encrypted.md](sending/01-send-encrypted.md) — You need a secure SRT uplink with passphrase-derived AES-CTR encryption negotiated at handshake.
- **Recipe 3: Mux to a file (no SRT, no transport)** — [03-mux-to-file.md](sending/03-mux-to-file.md) — You want the muxer's output without any networking — building test fixtures, validating output against TSDuck/ffprobe, or running an offline pipeline.
- **Recipe 8: Use a custom (non-SRT) transport** — [08-custom-transport.md](sending/08-custom-transport.md) — The sender shells fit but the wire isn't SRT — UDP, file, in-memory test harness, your own protocol.
- **Recipe 9a: Send MPEG-TS over UDP** — [udp.md](sending/udp.md) — Raw UDP unicast or multicast transport — lowest-common-denominator for compatibility with ffmpeg, VLC, and STANAG 4609 receivers.
- **Recipe 9b: Send MPEG-TS over TCP** — [tcp.md](sending/tcp.md) — Reliable-bytestream sibling of UDP, with optional TLS via rustls; pairs with `ffmpeg -listen 1 -i tcp://...` on the receiver side.
- **Recipe 9c: Publish MPEG-TS as an HLS stream** — [hls.md](sending/hls.md) — Built-in HTTP server serves `.ts` segments + `.m3u8` playlist; LIVE/EVENT/VOD modes; optional Basic auth + HTTPS via rustls. **Experimental, Rust-only — not in the published wheels/JAR; see [deferred-features.md](/docs/project/deferred-features.md).**
- **Recipe 9d: Send MPEG-TS over RIST** — [rist.md](sending/rist.md) — VideoLAN librist 0.2.16 bindings; ARQ-recovered UDP with Simple + Main profiles and AES-128/192/256 PSK encryption. Sender side.
- **Recipe 9: Mux H.265 + sync KLV** — [09-mux-h265-with-klv.md](sending/09-mux-h265-with-klv.md) — The encoder produces HEVC, or the receiver requires strict ST 1402 sync metadata (PMT stream_type 0x15) instead of the default async private-data shape.
- **Recipe 11: Open a sender from an `srt://...?...` URL** — [11-sender-from-url.md](sending/11-sender-from-url.md) — The connection target and tuning live in deployment config files or are passed in by an orchestrator.
- **Recipe 15: Label EO + IR + KLV streams in a multi-stream program** — [15-mux-eo-ir-klv.md](sending/15-mux-eo-ir-klv.md) — Multi-stream programs (Path 3) carry several PIDs in one program; per-stream PMT descriptors let receivers (TSDuck, ffprobe, our `Demuxer`) render which PID is which.
- **Recipe 16: Repack two single-program inputs into one multi-program TS** — [16-repack-multi-program.md](sending/16-repack-multi-program.md) — You have two independent (EO + IR + KLV) feeds and need to ship them through one SRT socket without forcing each to its own UDP port.
- **Recipe 19: Mux audio + video + KLV in a single program** — [19-mux-audio-video-klv.md](sending/19-mux-audio-video-klv.md) — Build a three-stream program where audio PTS-aligns with video for synchronized playback and KLV records emit on the same PCR clock.
- **Recipe 22: Streaming H.266 / VVC video with synchronous KLV metadata** — [22-mux-h266-with-klv.md](sending/22-mux-h266-with-klv.md) — The encoder produces H.266 (VVC) and the receiver requires strict ST 1402 sync KLV metadata.
- **Recipe 23: Streaming AV1 video with KLV metadata** — [23-mux-av1-with-klv.md](sending/23-mux-av1-with-klv.md) — The encoder produces AV1 — note OBU framing replaces Annex-B NAL framing.

### 📡 Receiving — consume a TS stream (includes KLV-to-video pairing)

- **Recipe 4: Relay a captured `.ts` file over SRT** — [04-relay-file-to-srt.md](receiving/04-relay-file-to-srt.md) — You have a `.ts` capture you want to replay over SRT — regression-testing receivers, rebroadcasting an archive, exercising a downstream pipeline.
- **Recipe 4a: Receive MPEG-TS over UDP** — [udp.md](receiving/udp.md) — Raw UDP unicast or multicast receiver — ingest from ffmpeg, VLC, or STANAG 4609 senders.
- **Recipe 4b: Receive MPEG-TS over TCP** — [tcp.md](receiving/tcp.md) — Reliable receiver — accept inbound caller (listener) or connect to a producer (caller-side receive). TLS supported via `tcps://`.
- **Recipe 4c: Receive MPEG-TS over RIST** — [rist.md](receiving/rist.md) — Receiver side of Recipe 9d. Bind URL form (`rist://@host:port`) with optional AES PSK decryption.
- **Recipe 5: Receive into a file** — [05-receive-to-file.md](receiving/05-receive-to-file.md) — Archiving a stream or building a test fixture from a live producer.
- **Recipe 12: Pair sync-KLV with video AUs by nearest PTS** — [12-pair-klv-by-pts.md](receiving/12-pair-klv-by-pts.md) — An encoder emits sync-KLV synchronized to video frames (one KLV per frame, KLV PES PTS = frame PTS) and you want to consume frame + telemetry as a paired record.
- **Recipe 13: Sample-and-hold async-KLV against video frames** — [13-sample-hold-klv.md](receiving/13-sample-hold-klv.md) — KLV is emitted independently of video — typically 1–10 Hz async metadata against 25–60 fps video.
- **Recipe 14: EO + IR sensor pair with shared async-KLV** — [14-eo-ir-shared-klv.md](receiving/14-eo-ir-shared-klv.md) — The platform carries two sensors (visible + thermal) and one async metadata stream serves both.
- **Recipe 21: Extract subtitle PES bytes from a captured `.ts` file** — [21-extract-subtitle-pes.md](receiving/21-extract-subtitle-pes.md) — Receive-side inspection — discover what subtitle codecs are in a capture and read the cue text.
- **Recipe 24: Pair sync-KLV with video AUs via `Pairer::with_config` (Realtime)** — [24-pairer-realtime.md](receiving/24-pairer-realtime.md) — You want the inline pattern from Recipe 12 expressed through the opt-in `Pairer` helper, with bounded history, telemetry counters, and typed projection structs.
- **Recipe 25: Pair sync-KLV in batch mode (`PairerMode::Buffered`)** — [25-pairer-batch.md](receiving/25-pairer-batch.md) — KLV PES is interleaved *after* its matching video PES (some encoders), and Realtime mode misses the pairing.
- **Recipe 26: Sample-and-hold async KLV via `Pairer::last_before_pts`** — [26-pairer-last-before-pts.md](receiving/26-pairer-last-before-pts.md) — Async-KLV streams where each video frame should attach the most recent KLV at `klv.pts <= video.pts`.
- **Recipe 27: EO + IR composition with shared async-KLV** — [27-eo-ir-shared-klv-pairer.md](receiving/27-eo-ir-shared-klv-pairer.md) — Two video PIDs share one async-KLV PID and you want telemetry counters + typed output projections per branch.
- **Recipe 34: Ingest H.264 from an RTSP camera and remux to MPEG-TS** — [34-recv-rtsp-h264-to-ts.md](receiving/34-recv-rtsp-h264-to-ts.md) — Camera exposes bare H.264-over-RTP (RFC 6184); gateway pattern re-muxes access units into MPEG-TS. Python-first; Rust example twin available.

### 🔑 KLV — encode and decode metadata directly

- **Recipe 6: Decode ST 0601 from a captured `.klv` blob** — [06-decode-st0601-blob.md](klv/06-decode-st0601-blob.md) — Validating producer output, building dashboards on captured data, or debugging a receiver.
- **Recipe 7: Encode ST 0601 from typed values** — [07-encode-st0601.md](klv/07-encode-st0601.md) — Synthesizing KLV for tests, generating fixtures, or translating from a different metadata format in a gateway.
- **Recipe 28: Decode security metadata from an ST 0601 record** — [28-decode-security-metadata.md](klv/28-decode-security-metadata.md) — ST 0601 Tag 48 (Security Local Set) is populated and you need ST 0102 classification, country codes, and version info.
- **Recipe 30: Decode VMTI per-target detections from an ST 0601 stream** — [30-decode-vmti.md](klv/30-decode-vmti.md) — ISR capture analysis — surface detected/tracked targets per frame via Tag 74 (VMTI Local Set).

### 🎞 Codecs — parse video and audio elementary streams

- **Recipe 17: Extract video resolution and profile from a demuxed stream** — [17-extract-resolution-profile.md](codecs/17-extract-resolution-profile.md) — You need typed codec information (width, height, profile, level, frame rate, color) and are already demuxing the stream.
- **Recipe 18: Reconstitute Annex B parameter sets for decoder replay** — [18-reconstitute-annex-b.md](codecs/18-reconstitute-annex-b.md) — You need to hand SPS / PPS bytes to a hardware decoder, encoder re-init, or a library that expects Annex-B-framed codec configuration.
- **Recipe 29: Pull sample rate and channel count out of an audio stream** — [29-extract-audio-format.md](codecs/29-extract-audio-format.md) — Inspect a `.ts` file and report typed audio metadata (sample rate, channel count, codec layer/profile) per audio PID.
- **Recipe 32: Decode video frames in-memory with PyAV (Python)** — [32-decode-frames-pyav.md](codecs/32-decode-frames-pyav.md) — Decode frames straight from `DemuxEvent.Video.raw` (or processed `ev.parse()` NAL units) in the same demux pass that yields your KLV — no file re-open, no OpenCV. Includes gapless windowed (time-slice) decode. Python-only (PyAV).

### ⚙ Operations — lifecycle, stats, shutdown, fixtures

- **Recipe 2: Survive a flaky transport with reconnect + gap buffer** — [02-managed-transport-reconnect.md](operations/02-managed-transport-reconnect.md) — The wire is lossy — radio links, NAT timeouts, listener restarts.
- **Recipe 10: Print live `Stats` from a sender** — [10-print-live-stats.md](operations/10-print-live-stats.md) — Building an operational dashboard, instrumenting a sender for production telemetry, or debugging packet loss in the field.
- **Recipe 20: Inject WebVTT POI cues into a live MPEG-TS uplink** — [20-inject-webvtt-cues.md](operations/20-inject-webvtt-cues.md) — A sensor/orchestrator wants to mark Points of Interest in a live SRT/TS stream so the downstream HLS player can render them as captions.
- **Recipe 31: Graceful shutdown from another thread via `SrtCancelHandle`** — [31-graceful-shutdown.md](operations/31-graceful-shutdown.md) — The main thread is parked in `send_*` / `recv_*` and a sibling thread (signal handler, watchdog, FFI lifecycle observer) needs to wake it.
- **Recipe: Capture a regression fixture from a corpus `.ts` file** — [99-capture-regression-fixture.md](operations/99-capture-regression-fixture.md) — The gitignored corpus surfaces a parser or demuxer bug and you want to preserve a minimal reproducer as a committed regression test.

## Browse by example program

Only examples explicitly invoked via `cargo run -p tst-examples --example <name>` in a recipe body are listed here. Examples referenced as runnable file paths (most of the muxing / sending recipes) appear in the recipe's "Related" header — browse the [examples/](../../examples/) tree for the full catalog.

| Example | Recipe(s) |
|---|---|
| `decode_vmti_metadata` | [Recipe 30](klv/30-decode-vmti.md) |
| `demux_subtitle_file` | [Recipe 21](receiving/21-extract-subtitle-pes.md) |
| `mux_dual_camera` | [Recipe 15](sending/15-mux-eo-ir-klv.md) |
| `mux_with_webvtt_subtitles` | [Recipe 20](operations/20-inject-webvtt-cues.md) |
| `pair_klv_pipeline` | [Recipe 24](receiving/24-pairer-realtime.md) |
| `parse_audio_frames` | [Recipe 29](codecs/29-extract-audio-format.md) |
