# Rust bindings

> **Who this is for:** You write Rust and want to add MPEG-TS + KLV + SRT
> to your application.

> **You will learn:**
> - How to add the ts-transformer crates to your `Cargo.toml`
> - How to mux H.264 video + KLV into a `.ts` file with ~10 lines of code
> - How to send the result over an SRT socket
> - How to demux a `.ts` file and dispatch typed `DemuxEvent` items
> - The role of feature flags (`mbedtls`, `file`) and the workspace MSRV
> - The Rust-specific gotchas: `#[non_exhaustive]` enums, SRT init, reconnect
>   wrappers, and when to pick `MuxSender` vs `Sender`
> - Where to find the deep guides for each subsystem

## Install

When you want the full sender + receiver pipeline (mux/demux + SRT), pull
the three top-level crates in:

```toml
[dependencies]
tst-core     = { git = "https://github.com/aklofas/ts-transformer" }  # MPEG-TS mux/demux + KLV + codec parsers
tst-pipeline = { git = "https://github.com/aklofas/ts-transformer" }  # Sender / Receiver / MuxSender / DemuxReceiver shells
tst-srt      = { git = "https://github.com/aklofas/ts-transformer" }  # SRT transport
```

When you only need to inspect or build `.ts` bytes (no live transport),
`tst-core` alone is enough — it has no SRT dependency and skips the
libsrt / mbedTLS compile step entirely.

**MSRV:** Rust **1.85** (workspace-pinned via `rust-toolchain.toml`).
Running `cargo` inside the workspace auto-uses 1.85 via rustup.

**Feature flags worth knowing:**

| Crate         | Feature   | Default | Effect                                                           |
| ------------- | --------- | ------- | ---------------------------------------------------------------- |
| `srt-sys`     | `mbedtls` | on      | Vendored mbedTLS, `USE_ENCLIB=mbedtls`. Disable for unencrypted. |
| `tst-srt`     | `mbedtls` | on      | Propagates to `srt-sys/mbedtls`.                                 |
| `tst-srt`     | `log`     | on      | Forwards libsrt's internal logging through the `log` facade.     |
| `tst-core`    | `file`    | on      | Gates file I/O helpers. Disable for embedded targets without `std::fs`. |

A clean rebuild compiles libsrt 1.5.5 and mbedTLS 3.6.6 from vendored
submodules — expect **3–5 minutes** on a cold cache, seconds when warm.
Force the vendored path with `SRT_FORCE_VENDORED=1` (otherwise the build
script tries `pkg-config srt ≥ 1.5.0` first).

For the per-target support matrix, see
[`/docs/reference/compatibility.md`](/docs/reference/compatibility.md).

## Hello world

The smallest useful thing: mux one H.264 access unit + one KLV blob into
188-byte TS packets, entirely in memory — no SRT, no peer, no file.

```rust
use tst_core::mpegts::common::Pts90khz;
use tst_core::mpegts::mux::{Muxer, MuxerConfig};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Default config: one program, H.264 video on PID 0x1011, async KLV on 0x1031.
    let mut muxer = Muxer::new(MuxerConfig::default())?;

    let nal = [0x00, 0x00, 0x00, 0x01, 0x65, 0xA5, 0xA5, 0xA5]; // minimal Annex-B IDR
    muxer.push_video(&nal, Pts90khz::new(0), /* key_frame */ true)?;
    muxer.push_klv(&[0x06, 0x0E, 0x2B, 0x34, 0xDE, 0xAD, 0xBE, 0xEF], Pts90khz::new(0), 0x00)?;

    let mut buf = [0u8; 1316];
    let n = muxer.pull(&mut buf);
    println!("muxed {n} bytes ({} TS packets)", n / 188);
    Ok(())
}
```

That's the whole shape: build a `Muxer`, push typed payloads, pull TS
bytes. Every other sender variant in this library is sugar over the same
push/pull contract.

## First send

When you want to ship those TS bytes over SRT to a peer, compose the
muxer with an `SrtTransport` via a `MuxSender`. The shell owns
synchronization between push and send, handles transient transport
failures, and gives you a single `send_video` / `send_klv` API:

```rust
use std::time::Duration;
use tst_core::mpegts::common::Pts90khz;
use tst_core::mpegts::mux::MuxerConfig;
use tst_pipeline::MuxSender;
use tst_srt::{SocketBuilder, SrtTransport};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Open an SRT socket to the peer. 120 ms latency is a reasonable
    //    LAN/regional WAN default; bump for transcontinental / cellular paths.
    let mut sb = SocketBuilder::new();
    sb.latency_ms(120);
    sb.recv_timeout(Duration::from_secs(5));
    let socket = sb.connect("127.0.0.1:9000")?;
    let transport = SrtTransport::new(socket);

    // 2. Wrap muxer + transport. Default config = 1 program, H.264 + async KLV.
    let mut sender: MuxSender<SrtTransport> =
        MuxSender::new(MuxerConfig::default(), transport)?;

    // 3. Push payloads. Each push muxes into TS packets and ships them.
    let nal = [0x00, 0x00, 0x00, 0x01, 0x65, 0xA5, 0xA5, 0xA5];
    sender.send_video(&nal, Pts90khz::new(0), /* key_frame */ true)?;
    sender.send_klv(&[0x06, 0x0E, 0x2B, 0x34, 0xDE, 0xAD, 0xBE, 0xEF], Pts90khz::new(0))?;

    sender.close()?;
    Ok(())
}
```

On the receiver side, run something like:

```bash
srt-live-transmit srt://:9000 file:///tmp/out.ts
```

For the full runnable version with synthetic frames + commentary on every
config knob:

```bash
cargo run -p tst-examples --example send_pipeline_to_socket -- 127.0.0.1:9000
```

Other send-side examples worth knowing:

- [`examples/sending/encrypted_send_recv.rs`](/examples/sending/encrypted_send_recv.rs) — AES passphrase encryption end to end.
- [`examples/sending/srt_serve_ts_file.rs`](/examples/sending/srt_serve_ts_file.rs) — listen mode (peer dials in).
- [`examples/sending/sender_from_url.rs`](/examples/sending/sender_from_url.rs) — config via `srt://host:port?key=value`.
- [`examples/sending/custom_transport.rs`](/examples/sending/custom_transport.rs) — bring your own `Transport` impl (UDP, file, etc.).

See [`/docs/guides/mpegts-mux.md`](/docs/guides/mpegts-mux.md) for the full
`MuxerConfig` surface, and [`/docs/guides/pipeline.md`](/docs/guides/pipeline.md)
for picking among `MuxSender` / `Sender` / `RawSender`.

## First receive

Pulling typed events out of a `.ts` file (or live SRT stream) takes the
same shape: build a `Demuxer`, feed bytes, dispatch by event variant.

```rust
use std::env;
use std::fs;
use tst_core::mpegts::demux::{DemuxEvent, Demuxer};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = env::args().nth(1).ok_or("usage: <file.ts>")?;
    let bytes = fs::read(&path)?;

    // Lenient by default. The demuxer keeps going past every recoverable
    // problem and surfaces what it found as `NonConformant` events. For
    // hard-fail behavior, swap to `DemuxerBuilder::new().strict(...).build()`.
    let mut d = Demuxer::new();
    d.push(&bytes);

    while let Some(event) = d.poll_event() {
        match event {
            DemuxEvent::ProgramMap { programs, .. } => {
                println!("PSI: {} programs", programs.len());
            }
            DemuxEvent::Sample(s) => {
                println!("Sample pid=0x{:04X} pts={:?}", s.pid, s.pts);
            }
            DemuxEvent::Metadata(m) => {
                println!("Metadata pid=0x{:04X} kind={:?}", m.pid, m.kind);
            }
            DemuxEvent::Discontinuity(d) => eprintln!("Discontinuity: {d:?}"),
            DemuxEvent::NonConformant(nc) => eprintln!("NonConformant: {nc:?}"),
            // #[non_exhaustive] enum — wildcard arm required.
            _ => {}
        }
    }
    Ok(())
}
```

Run the full version against any `.ts` file:

```bash
cargo run -p tst-examples --example demux_to_events -- /path/to/capture.ts
```

For the live SRT-side analogue, see
[`examples/receiving/srt_recv_typed.rs`](/examples/receiving/srt_recv_typed.rs) —
same event shape, but reading from a connected SRT socket instead of a
file. To dump bytes straight to a file:
[`examples/receiving/srt_listener_to_file.rs`](/examples/receiving/srt_listener_to_file.rs).

The full demuxer contract — strict-mode ladder, override surface, AU-cell
unwrap behavior, decoupled-pairing rationale — is in
[`/docs/guides/mpegts-demux.md`](/docs/guides/mpegts-demux.md).

## Language-specific gotchas

**`#[non_exhaustive]` enums require a wildcard arm.** `DemuxEvent`,
`MuxError`, `DemuxError`, `NonConformantIssue`, and many other public
enums in this workspace are marked `#[non_exhaustive]` so new variants
land without a major version bump. Your `match` arms must include a
`_ => { ... }` catch-all; the compiler error is explicit when you
forget. The current variant count is ratcheted in CI (see
`BASELINE=162` in `.github/workflows/ci.yml`).

**SRT initialization is automatic.** `srt-sys` calls `srt_startup` /
`srt_cleanup` on your behalf — don't call them manually. Cleanup runs
at process exit. If you build with `--no-default-features`, encryption
(mbedTLS) is omitted but the libsrt init / teardown path is unchanged.

**Pick the right sender shell.** `MuxSender<T>` is the canonical choice
when you have raw encoded video NALs + KLV records and want this library
to mux. Use `Sender<T>` (raw TS-bytes-through-transport) only when you
already have pre-muxed TS bytes from elsewhere (e.g. ffmpeg pipe).
`RawSender<T>` is the byte-blind one-message-per-call primitive — rarely
the right choice unless you're building your own framing layer.

**Reconnect is opt-in via wrappers.** A bare `SrtTransport` connects
once and fails hard on disconnect. To get exponential-backoff reconnect
+ a configurable gap buffer, wrap with `ManagedTransport<T>` on the
send side or `ManagedRecvTransport<T>` / `ManagedDemuxReceiver<T>` on
the receive side. The reconnect policy is a single `ReconnectPolicy`
struct — see [`/docs/guides/pipeline.md`](/docs/guides/pipeline.md) for
the full state machine.

**Feature flag interactions.** `--no-default-features` on `tst-srt`
disables mbedTLS and turns the libsrt build into unencrypted-only.
`--no-default-features --features file` on `tst-core` keeps file I/O
helpers while dropping the (currently-empty for `tst-core`) default set
— relevant for embedded `no_std`-ish targets. The two flag sets are
independent.

**Builders use bind-then-step, not single-chain.** `SocketBuilder` and
`ListenerBuilder` mutators take `&mut self` but their terminal methods
(`connect`, `bind`) take `&self`. A single fluent chain off a temporary
dangles. Always bind the builder to a local first:

```rust
let mut sb = SocketBuilder::new();
sb.latency_ms(120);             // &mut self
let socket = sb.connect(addr)?; // &self
```

**Pairing KLV to video.** The demuxer emits KLV and video as independent
events on the same PTS clock; aligning them is the consumer's job. The
`Pairer` shell in `tst-pipeline::pairing` is the standard solution —
configurable window, drop policy, and event-order preservation. See the
[`pairing/` examples directory](/examples/pairing/) and
[`/docs/cookbook/index.md`](/docs/cookbook/index.md).

## Where this binding differs from the Rust core

You're already at the canonical surface — there's no Rust-specific
deviation to document. Everything visible from the other language pages
exists here, in its richest form.

If you're integrating with another language, the dedicated pages call
out each binding's deviations relative to this surface:

- **C / C++:** [`/docs/languages/c.md`](/docs/languages/c.md) — opaque
  handles, libsrt-style negative error codes + thread-local last-error,
  ABI versioning.
- **Python:** [`/docs/languages/python.md`](/docs/languages/python.md) —
  file-I/O-only v1 surface (no live transport yet), `match`-friendly
  `DemuxEvent` subclasses, pandas / NumPy adapters, GIL release on long
  calls.

The "Where this binding differs" section on each of those pages is the
authoritative gap list. Anything not called out there matches Rust 1:1.

## Where to go next

- [`/docs/start/concepts.md`](/docs/start/concepts.md) — the conceptual
  model (mux/demux, KLV, transport, pipeline shells) before any code.
- [`/docs/cookbook/index.md`](/docs/cookbook/index.md) — recipes keyed to runnable
  examples for the most common patterns.
- [`/docs/guides/srt.md`](/docs/guides/srt.md) — full SRT surface:
  encryption, latency, stats, error model, URL parsing.
- [`/docs/guides/klv.md`](/docs/guides/klv.md) — generic KLV substrate
  plus typed ST 0601 / ST 0102 / ST 0605 / ST 0903 layers.
- [`/docs/guides/codec.md`](/docs/guides/codec.md) — stateless H.264 /
  H.265 / H.266 / AV1 parameter-set parsers off demuxer NAL / OBU bytes.
- [`/docs/troubleshooting.md`](/docs/troubleshooting.md) — symptom →
  diagnosis → fix for build, connection, KLV, framing, and reconnect
  issues.
- [`/docs/reference/compatibility.md`](/docs/reference/compatibility.md)
  — feature-by-feature support matrix (SRT options, MISB specs, typed
  ST 0601 items, codecs, platforms).
