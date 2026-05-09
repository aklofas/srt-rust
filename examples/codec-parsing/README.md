# Codec parsing

Pull video / audio / subtitle parameter sets out of demuxed streams.
Six examples; the first three are progressively richer parsers, the
last three are extraction recipes that feed downstream tooling.

## 1. `parse_video_parameters.rs` — multi-codec sweep

```sh
cargo run -p tst-examples --example parse_video_parameters -- <file.ts>
```

Walks every video stream in a `.ts` file, dispatches on `VideoCodec`,
prints resolution + profile + level for H.264 and H.265 in one pass.
The "what's in this capture?" diagnostic.

Cookbook: [§17 — Extract video resolution and profile from a demuxed stream](../../docs/cookbook.md#17-extract-video-resolution-and-profile-from-a-demuxed-stream).

## 2. `parse_audio_frames.rs` — MP2 / AAC frame iteration

```sh
cargo run -p tst-examples --example parse_audio_frames -- path/to/some.ts
```

Diff from §1: audio side. Lazy stateless iterator over MPEG audio
(MP1 / MP2 / MP3 / MPEG-2 LSF / MPEG-2.5) and AAC ADTS frames; pulls
sample rate, channel count, frame size out of the headers.

Cookbook: [§29 — Pull sample rate and channel count out of an audio stream](../../docs/cookbook.md#29-pull-sample-rate-and-channel-count-out-of-an-audio-stream).

## 3. `extract_video_au.rs` — extract video AUs to disk

```sh
cargo run -p tst-examples --example extract_video_au -- <input.ts> [out_dir]
```

Diff from §1: instead of just printing parameters, write each access
unit to its own file. Useful when feeding a downstream decoder /
analyzer that wants Annex B or OBU bytes. Multi-codec: H.264 / H.265 /
H.266 / AV1 all handled.

## 4. `extract_h265_sps_to_rbsp.rs` — emulation-prevention removal (HEVC)

```sh
cargo run -p tst-examples --example extract_h265_sps_to_rbsp -- input.265 output.bin
```

Strip `0x000003` emulation prevention bytes from a NAL unit, write the
raw RBSP. The minimal recipe for "I have an Annex B byte stream and
want the RBSP for parameter-set parsing." H.265-specific.

Cookbook: [§18 — Reconstitute Annex B parameter sets for decoder replay](../../docs/cookbook.md#18-reconstitute-annex-b-parameter-sets-for-decoder-replay).

## 5. `extract_h266_sps_to_rbsp.rs` — same recipe, H.266 / VVC

```sh
cargo run -p tst-examples --example extract_h266_sps_to_rbsp -- input.266 output.bin
```

Diff from §4: VVC-specific NAL header layout (2 bytes instead of 1)
but same emulation-prevention strip.

## 6. `demux_subtitle_file.rs` — subtitle codec discrimination

```sh
cargo run -p tst-examples --example demux_subtitle_file -- input.ts
```

DVB-sub / DVB-teletext / CEA-708 / WebVTT-in-TS all share
`stream_type` 0x06; the right descriptor distinguishes them. This
example walks every subtitle stream in a capture and reports which
codec each one carries.

Cookbook: [§21 — Extract subtitle PES bytes from a captured `.ts` file](../../docs/cookbook.md#21-extract-subtitle-pes-bytes-from-a-captured-ts-file).
