# ts-transformer

A fast, embeddable Rust library for MPEG-TS video + KLV metadata streaming over SRT — with C, Python, and JVM (planned) bindings.

*The Swiss Army Knife for MPEG-TS streams.*

[![CI](https://github.com/aklofas/ts-transformer/actions/workflows/ci.yml/badge.svg)](https://github.com/aklofas/ts-transformer/actions/workflows/ci.yml)
[![PyPI](https://img.shields.io/pypi/v/tstrans.svg)](https://pypi.org/project/tstrans/)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)
[![MSRV: 1.85](https://img.shields.io/badge/MSRV-1.85-blue.svg)](rust-toolchain.toml)

| Language | Status | Entry point |
|---|---|---|
| **Rust** | Shipping | [`docs/languages/rust.md`](docs/languages/rust.md) |
| **C** | Shipping (ABI 0.5) | [`docs/languages/c.md`](docs/languages/c.md) |
| **Python** | Shipping (`tstrans` on PyPI; includes `tstrans.rtp` RTP + RTSP) | [`docs/languages/python.md`](docs/languages/python.md) |
| **JVM** | Planned (`tst-jni` next) | [roadmap](docs/project/deferred-features.md) |

## In 30 seconds

Mux one H.264 access unit + one KLV blob into 188-byte MPEG-TS packets — no SRT peer, no file:

```rust
use tst_core::mpegts::mux::{Muxer, MuxerConfig};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Default config: one program, H.264 video on PID 0x1011, KLV on 0x1031.
    let mut muxer = Muxer::new(MuxerConfig::default())?;

    let nal = [0x00, 0x00, 0x00, 0x01, 0x65, 0xA5, 0xA5, 0xA5];
    muxer.push_video(&nal, /* pts_90khz */ 0, /* key_frame */ true)?;
    muxer.push_klv(&[0x06, 0x0E, 0x2B, 0x34, 0xDE, 0xAD, 0xBE, 0xEF], 0, 0x00)?;

    let mut buf = [0u8; 1316];
    let n = muxer.pull(&mut buf);
    println!("muxed {n} bytes ({} TS packets)", n / 188);
    Ok(())
}
```

Python equivalent: `pip install tstrans` — see [`docs/languages/python.md`](docs/languages/python.md).
**Run end-to-end (mux → SRT → demux) in 10 minutes:** [`docs/start/quickstart.md`](docs/start/quickstart.md).

## What this is — and what it isn't

**Use ts-transformer when you need to:**
- Stream live H.264 / H.265 / H.266 / AV1 video over an unreliable network with reconnect, encryption, and stats.
- Carry typed MISB ST 0601 / 0102 / 0605 / 0903 KLV metadata in the same transport stream as the video.
- Decode an existing `.ts` file or live stream into typed `DemuxEvent` items (KLV records, NAL units, audio frames, subtitles).
- Embed all of the above in a Rust, C, or Python app via a stable surface.

**Look elsewhere if you need:**
- **RTMP** or **WebRTC** transports — not on the roadmap. (SRT is shipping today; **RTP** and **raw TCP / UDP** are in active development; **RIST** may follow.)
- A different container — we are **MPEG-TS only** (no MP4, fMP4, HLS, DASH; pair us with FFmpeg for repackaging).
- Arbitrary metadata schemas — we are **MISB KLV only**.
- Video encoders or decoders — we mux + demux the wire format; pair with x264 / x265 / FFmpeg / NVENC / PyAV / a hardware codec for the actual codec work.
- A turnkey server — we are a **library**, not MediaMTX / Nimble / Wowza.

## Scope

| | |
|---|---|
| **Container** | MPEG-TS (single-program, multi-program; PAT / PMT / PCR auto-generated) |
| **Video** | H.264, H.265, H.266 / VVC, AV1 |
| **Audio** | AAC (ADTS + LATM), MPEG-2 Audio (MP2 / MP3), AC-3, EAC-3 |
| **Subtitles** | DVB subtitling, DVB teletext, CEA-708, WebVTT-in-TS |
| **Metadata** | MISB ST 0601 (FMV), ST 0102 (security), ST 0605 (amend tags), ST 0903 (VMTI); ST 1402 carriage + H.222.0 §2.12.4.2 Metadata AU cells |
| **Transport** | SRT 1.5 (Haivision libsrt, vendored) shipping today; RTP and raw TCP / UDP in active development |
| **Encryption** | AES-128 / 192 / 256 over SRT via vendored mbedTLS 3.6 LTS, on by default |

Full feature-by-feature matrix in [`docs/reference/compatibility.md`](docs/reference/compatibility.md).

## Install

```bash
# Python (PyPI)
pip install tstrans                   # core
pip install tstrans[pandas]           # + DataFrame + NumPy adapters

# Rust (git, until first crates.io publish)
cargo add --git https://github.com/aklofas/ts-transformer tst-core
```

<details>
<summary>From source (Rust + C + vendored libsrt / mbedTLS)</summary>

```bash
git clone --recurse-submodules https://github.com/aklofas/ts-transformer.git
cd ts-transformer/ts-transformer

# Rust workspace
SRT_FORCE_VENDORED=1 cargo build --release

# C bindings (cdylib + staticlib + tstrans.h + tstrans.pc)
SRT_FORCE_VENDORED=1 cargo build --release -p tst-c
```

The build script compiles libsrt 1.5.5 and mbedTLS 3.6.x from vendored submodules — expect 3–5 minutes on cold cache, seconds when warm. See [`docs/languages/rust.md`](docs/languages/rust.md) for feature flags (`mbedtls`, `file`, etc.).

</details>

## Documentation

→ **[`docs/index.md`](docs/index.md)** — the docs landing page routes 5 reader audiences (cold-domain reader, evaluator, language integrator, domain expert, binding author).

| If you are… | Start here |
|---|---|
| New to MPEG-TS / KLV / SRT | [`docs/start/concepts.md`](docs/start/concepts.md) — plain-language explainers |
| Evaluating the library | [`docs/start/overview.md`](docs/start/overview.md) — what it does, what's in the box, what's not |
| Writing your first code | [`docs/start/quickstart.md`](docs/start/quickstart.md) — working mux + demux in 10 minutes |
| Building something real | [`docs/cookbook/index.md`](docs/cookbook/index.md) — 33 task-oriented recipes |
| Looking up a type or error | [`docs/reference/`](docs/reference/) — architecture, conventions, public-API policy |
| Wrapping for a new language | [`docs/reference/binding-authors.md`](docs/reference/binding-authors.md) — ABI, error mapping, stability tiers |

## Interoperates with

The wire format is standards-conformant; outputs are validated against **FFmpeg / ffprobe**, **TSDuck**, **GStreamer**, **VLC**, **mpv**, and reference MISB tooling. Use any of them as the encoder, decoder, validator, or display layer on either side of a ts-transformer pipeline.

## Status

Pre-1.0. Sender + receiver pipelines complete; **Linux x86_64 + Linux aarch64 are CI-gating**; macOS arm64 and Windows MSVC build + link verified, runtime test promotion tracked in [`docs/project/deferred-features.md`](docs/project/deferred-features.md). Hundreds of tests across both feature modes; 20 bash ratchets + `cargo public-api` baselines guard the surface on every commit.

Public API may change between pre-1.0 releases; all changes recorded in [`CHANGELOG.md`](CHANGELOG.md).

## Contributing

Issues and PRs welcome. See [`docs/reference/conventions.md`](docs/reference/conventions.md) for code style, commit message rules, and the public-API workflow.

## License

Licensed under **MIT** ([`LICENSE-MIT`](LICENSE-MIT)) or **Apache-2.0** ([`LICENSE-APACHE`](LICENSE-APACHE)) at your option.
