# ts-transformer

**Carry live video and MISB KLV telemetry across an unreliable link** — muxed into
MPEG-TS, streamed over UDP / TCP / RTP / SRT / RIST, with built-in reconnect,
encryption, and typed metadata decoding. Rust core; C, Python, and JVM bindings.

*The Swiss-Army knife for MPEG-TS + KLV streams.*

[![CI](https://github.com/aklofas/ts-transformer/actions/workflows/ci.yml/badge.svg)](https://github.com/aklofas/ts-transformer/actions/workflows/ci.yml)
[![PyPI](https://img.shields.io/pypi/v/tstrans.svg)](https://pypi.org/project/tstrans/)
[![Maven Central](https://img.shields.io/maven-central/v/org.tstrans/tstrans-jvm.svg)](https://central.sonatype.com/artifact/org.tstrans/tstrans-jvm)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)
[![MSRV: 1.85](https://img.shields.io/badge/MSRV-1.85-blue.svg)](rust-toolchain.toml)

| Language | Status | Start here |
|---|---|---|
| **Rust** | Shipping | [`docs/languages/rust.md`](docs/languages/rust.md) |
| **C** | Shipping · ABI 0.17 | [`docs/languages/c.md`](docs/languages/c.md) |
| **Python** | Shipping · `tstrans` on PyPI | [`docs/languages/python.md`](docs/languages/python.md) |
| **JVM** | Shipping · `org.tstrans:tstrans-jvm` on Maven Central | [`docs/languages/jvm.md`](docs/languages/jvm.md) |
| **Embedded (bare-metal / RTOS)** | Shipping · QEMU-gated `no_std` core + C staticlib | [`docs/languages/embedded.md`](docs/languages/embedded.md) |

## See it in 30 seconds

Mux one H.264 access unit + one KLV blob into 188-byte MPEG-TS packets — no network, no files:

```rust
use tst_core::mpegts::common::Pts90khz;
use tst_core::mpegts::mux::{Muxer, MuxerConfig};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Default config: one program, H.264 video on PID 0x1011, KLV on 0x1031.
    let mut muxer = Muxer::new(MuxerConfig::default())?;

    let nal = [0x00, 0x00, 0x00, 0x01, 0x65, 0xA5, 0xA5, 0xA5];
    muxer.push_video(&nal, Pts90khz::new(0), /* key_frame */ true)?;
    muxer.push_klv(&[0x06, 0x0E, 0x2B, 0x34, 0xDE, 0xAD, 0xBE, 0xEF], Pts90khz::new(0), 0x00)?;

    let mut buf = [0u8; 1316];
    let n = muxer.pull(&mut buf);
    println!("muxed {n} bytes ({} TS packets)", n / 188);
    Ok(())
}
```

Swap `Muxer` for `MuxSender` to push those packets straight onto an RTP / SRT / RIST link,
or `Demuxer` to pull typed events back out. The same surface ships in Python (`tstrans`),
C (`tstrans.h`), and the JVM (`org.tstrans`).

**Want the full mux → SRT → demux round-trip?** [`docs/start/quickstart.md`](docs/start/quickstart.md) gets you there in 10 minutes.

## What it's for — and what it isn't

**Reach for ts-transformer when you need to:**
- Stream live H.264 / H.265 / H.266 / AV1 over a lossy network with reconnect, encryption, and live stats.
- Carry typed MISB ST 0601 / 0102 / 0605 / 0903 KLV metadata in the *same* transport stream as the video.
- Decode a `.ts` file or live feed into typed `DemuxEvent` items — KLV records, NAL units, audio frames, subtitles.
- Embed any of the above in a Rust, C, Python, or JVM app behind a stable surface.

**Look elsewhere if you need:**
- **RTMP** or **WebRTC** — not on the roadmap. (UDP, raw TCP / TLS, RTP incl. RTSP client + server, SRT, and RIST all ship today.)
- A different container — we're **MPEG-TS only** (no MP4 / fMP4 / DASH; pair with FFmpeg to repackage).
- Arbitrary metadata schemas — we're **MISB KLV only**.
- An encoder or decoder — we handle the *wire format*; pair with x264 / x265 / FFmpeg / NVENC / PyAV for the codec work.
- A turnkey server — we're a **library**, not MediaMTX / Nimble / Wowza.

## Scope

| | |
|---|---|
| **Container** | MPEG-TS (single- and multi-program; PAT / PMT / PCR auto-generated) |
| **Video** | H.264, H.265, H.266 / VVC, AV1 |
| **Audio** | AAC (ADTS + LATM), MPEG-2 Audio (MP2 / MP3), AC-3 |
| **Subtitles** | DVB subtitling, DVB teletext, CEA-708, WebVTT-in-TS |
| **Metadata** | MISB ST 0601 (FMV), ST 0102 (security), ST 0605 (amend tags), ST 0903 (VMTI); ST 1402 carriage + H.222.0 §2.12.4.2 Metadata AU cells |
| **Transport** | UDP, raw TCP / TLS, RTP (incl. RTSP client + server), SRT 1.5 (Haivision libsrt, vendored), RIST (VideoLAN librist) — all shipping. HLS publisher is experimental and excluded from published artifacts (see [`docs/project/deferred-features.md`](docs/project/deferred-features.md)). |
| **Encryption** | AES-128 / 192 / 256 over SRT via vendored mbedTLS 3.6 LTS, on by default |

Full feature-by-feature matrix: [`docs/reference/compatibility.md`](docs/reference/compatibility.md).

## Install

`tstrans` is on **PyPI** and `org.tstrans:tstrans-jvm` is on **Maven Central**.
The Rust core and C bindings build from source:

```bash
git clone --recurse-submodules https://github.com/aklofas/ts-transformer.git
cd ts-transformer/ts-transformer

SRT_FORCE_VENDORED=1 cargo build --release            # Rust workspace
SRT_FORCE_VENDORED=1 cargo build --release -p tst-c   # C bindings → cdylib + staticlib + tstrans.h + tstrans.pc
```

The build vendors and compiles libsrt 1.5.5 + mbedTLS 3.6 LTS from submodules — ~3–5 minutes
cold, seconds warm. Feature flags (`mbedtls`, `file`, per-transport) are documented in
[`docs/languages/rust.md`](docs/languages/rust.md).

- **Python** — `pip install tstrans` (core) or `pip install tstrans[pandas]` (DataFrame + NumPy adapters). See [`docs/languages/python.md`](docs/languages/python.md).
- **JVM** — `org.tstrans:tstrans-jvm:0.2.0` on Maven Central. See [`docs/languages/jvm.md`](docs/languages/jvm.md).
- **Rust** — until the first crates.io publish: `cargo add --git https://github.com/aklofas/ts-transformer tst-core`.

## Documentation

→ **[`docs/index.md`](docs/index.md)** routes five reader audiences (cold-domain reader, evaluator,
language integrator, domain expert, binding author). Or jump straight in:

| If you are… | Start here |
|---|---|
| New to MPEG-TS / KLV / SRT | [`docs/start/concepts.md`](docs/start/concepts.md) — plain-language explainers |
| Evaluating the library | [`docs/start/overview.md`](docs/start/overview.md) — what it does, what's in the box, what's not |
| Writing your first code | [`docs/start/quickstart.md`](docs/start/quickstart.md) — working mux + demux in 10 minutes |
| Building something real | [`docs/cookbook/index.md`](docs/cookbook/index.md) — 40+ task-oriented recipes |
| Looking up a type or error | [`docs/reference/`](docs/reference/) — architecture, conventions, public-API policy |
| Wrapping for a new language | [`docs/reference/binding-authors.md`](docs/reference/binding-authors.md) — ABI, error mapping, stability tiers |

## Validated against

The wire format is standards-conformant, and outputs are checked against **FFmpeg / ffprobe**,
**TSDuck**, **GStreamer**, **VLC**, **mpv**, and reference MISB tooling. Use any of them as the
encoder, decoder, validator, or viewer on either side of a ts-transformer pipeline.

## Status

Pre-1.0. Sender + receiver pipelines are complete, and **all four Tier-1 targets gate CI** —
Linux x86_64, Linux aarch64, macOS arm64, and Windows MSVC (the RIST runtime test stays gated
on Windows; see [`docs/project/deferred-features.md`](docs/project/deferred-features.md)).
Hundreds of tests run across both feature modes, with bash ratchets and `cargo public-api`
baselines guarding the surface on every commit.

Public API may change between pre-1.0 releases; everything is recorded in [`CHANGELOG.md`](CHANGELOG.md).

## Contributing

Issues and PRs welcome — see [`docs/reference/conventions.md`](docs/reference/conventions.md)
for code style, commit-message rules, and the public-API workflow.

## License

Licensed under **MIT** ([`LICENSE-MIT`](LICENSE-MIT)) or **Apache-2.0** ([`LICENSE-APACHE`](LICENSE-APACHE)),
at your option.
