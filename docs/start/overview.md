# What ts-transformer is

> **Who this is for:** You've landed on the project and want to know in five minutes what it does, what it doesn't do, and whether to keep reading.

> **You will learn:**
> - What ts-transformer streams and what it doesn't
> - The three places this library sits in a system: source, middle, display
> - What protocols and metadata standards are involved (in plain terms)
> - What's in the box today (Rust + C + Python + JVM-soon)
> - What's not in the box, with links to deferred-features
> - Where to read next based on what you're doing

## What it streams

ts-transformer streams **live video and metadata over an unreliable network**. The classic shape:

> A camera on a sensor platform — a drone, an aircraft, a helicopter with an EO/IR turret, a ground vehicle, or a fixed installation — encodes H.264 or H.265 video. Alongside each frame, the platform's sensors emit telemetry: latitude, longitude, altitude, heading, sensor pointing angles. All of it gets multiplexed into one byte stream, encrypted, and pushed across a flaky network — satellite, cellular, mesh radio — to a ground station or cloud ingest. On the other end, a viewer or a processing pipeline pulls it apart: video to a decoder, metadata to a database, both stamped with the same timestamp so they line up on a map.

That's the shape. ts-transformer handles the encoding side, the metadata multiplexing, the wire format, the reconnect, and the encryption. **Your code is the camera, the metadata source, the viewer, or the processing pipeline.** The library is the plumbing between them.

## Where it sits

The library lives in one of three places in your system:

- **At the source** — you're producing the stream. Encoder on a drone, a gimbal platform, a ground encoder. You feed in encoded video access units + typed KLV records; the library mux's them into MPEG-TS and sends over SRT.
- **In the middle** — you're processing, indexing, or relaying. Cloud ingest that extracts KLV into a database, a service that transcodes for HLS, a relay that re-broadcasts to multiple consumers, an archive that records to disk.
- **At the display** — you're showing the stream to a human. Desktop player, mobile viewer, situational-awareness UI that overlays the KLV-derived lat/lon on a map.

Each placement uses the same primitives differently. The [`guides/`](/docs/guides/) describe the primitives; the [cookbook](/docs/cookbook/index.md) shows the placement-specific recipes.

## What's in the box

- **MPEG-TS mux + demux** — the Rust core (`tst-core`, `tst-pipeline`, `tst-srt`). Single-program or multi-program TS; auto-PCR insertion; PAT/PMT generation; the full ST 1402 KLV-in-TS multiplexing pipeline.
- **SRT transport** — vendored libsrt 1.5.5; mbedTLS 3.6.x LTS encryption ON by default with AES-128/192/256.
- **MISB KLV** — typed encode + decode for ST 0601 (Full Motion Video FMV), ST 0102 (Security Metadata), ST 0605 (Amend Tags), ST 0903 (VMTI per-target detections). H.222.0 §2.12.4.2 Metadata AU cell wrapping for synchronous KLV streams.
- **Video codecs** — H.264, H.265, H.266/VVC, AV1. NAL/OBU parsers; SPS/PPS/VPS extraction; slice-header-light parsers for resolution + profile.
- **Audio codecs** — AAC (ADTS + LATM), MPEG-2 Audio (MP2/MP3), AC-3, EAC-3. Frame-level parsers expose sample rate / channel count.
- **Subtitles** — DVB subtitling, DVB teletext, CEA-708, WebVTT-in-TS.
- **C bindings** (`tst-c`) — `cdylib` + `staticlib`, `tstrans.h` via cbindgen, `tstrans.pc` for pkg-config. Stable ABI versioned `TST_ABI_VERSION_MAJOR/MINOR`.
- **Python bindings** (`tst-py`, distributed as `tstrans` on PyPI) — file inspection and offline construction of `.ts` files; typed KLV decode/encode; typed `Sample.payload` (NalUnit / Obu / AdtsFrame / Mpeg2AudioFrame); optional pandas + NumPy adapters via `pip install tstrans[pandas]`.
- **JVM bindings** — `tst-jni`, planned, next on roadmap.

## What's NOT in the box

ts-transformer is intentionally narrow. If you need any of these, look elsewhere or pair the library with a complementary tool:

- **Other containers** — MPEG-TS only. No MP4, MKV, fMP4, HLS, DASH. (You can transcode + repackage on the receiver side using FFmpeg, GStreamer, etc.)
- **Other transports** — SRT 1.5 (Haivision libsrt, vendored) ships today. **RTP** and **raw TCP / UDP** are in active development. **RIST** may follow. **RTMP** and **WebRTC** are not on the roadmap.
- **Other metadata formats** — MISB KLV only. No arbitrary user data, no raw timestamps, no proprietary metadata schemas. See [`project/deferred-features.md`](/docs/project/deferred-features.md) for what's deferred.
- **Video encoding / decoding** — wire-format only. You bring the encoded NAL units / OBU frames; the library multiplexes them. Pair with x264 / x265 / FFmpeg / NVENC / GStreamer for the actual encode side; PyAV / FFmpeg / a hardware decoder for display.
- **Live SRT in Python** — file I/O only in v1. Live SRT lands in `tstrans` v2 (Rust core is ready; Python wrap is the work).
- **GUI** — no display layer. Pair with VLC, mpv, MPV.js, a custom decoder + framebuffer, or any other player that consumes MPEG-TS.

For the full feature-by-feature support matrix, see [`reference/compatibility.md`](/docs/reference/compatibility.md). For things deferred with a rationale + revisit trigger, see [`project/deferred-features.md`](/docs/project/deferred-features.md).

## What to read next

- **Brand new to MPEG-TS / KLV / SRT?** → [`start/concepts.md`](/docs/start/concepts.md) explains the domain vocabulary.
- **Ready to write code?** → [`start/quickstart.md`](/docs/start/quickstart.md).
- **Picking a language?** → [Language decision table](/docs/index.md#which-language-should-i-pick) on the landing page.
- **Want a deep dive?** → [`guides/`](/docs/guides/) — one per topic (mpegts-mux, mpegts-demux, srt, klv, codec, pipeline).
- **Need a code recipe?** → [Cookbook](/docs/cookbook/index.md).
