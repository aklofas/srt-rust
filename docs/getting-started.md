# Getting Started

## Audience and time budget

If you're new to `ts-transformer` and want to send and receive bytes in
10 minutes, start here. For deeper context, see
[architecture.md](architecture.md).

This guide walks through three runnable snippets in order: a raw send,
a raw receive, and a video-frame send through the pipeline shell. By
the end you'll have a sender and receiver talking over loopback and a
finished `.ts` file on disk.

## Prerequisites

- Rust 1.85+ via rustup. Check: `rustc --version`. The repo's
  `rust-toolchain.toml` pins to 1.85 for local development.
- C/C++ toolchain (`cmake`, `pkg-config`, `python3`, `build-essential`).
  Required because `srt-sys` and `tst-core` build vendored libsrt and
  mbedTLS from source.
- Debian/Ubuntu:
  `sudo apt-get install -y build-essential cmake pkg-config python3`.
- macOS: `brew install cmake pkg-config` (`python3` is pre-installed).

## Get the code

```bash
git clone --recurse-submodules https://github.com/aklofas/ts-transformer.git
cd ts-transformer
```

If you cloned without `--recurse-submodules`:

```bash
git submodule update --init --recursive
```

The submodules are `vendor/srt` (libsrt 1.5.5) and `vendor/mbedtls`
(mbedTLS 3.6.x LTS). Both are required for the default build.

## Add it to your project

Until `ts-transformer` is published to crates.io, depend on it via git:

```toml
[dependencies]
tst-core = { git = "https://github.com/aklofas/ts-transformer" }
```

Note on cold builds: the first build compiles libsrt + mbedTLS from
source and takes ~3-5 minutes. Pass `--no-default-features` on
`tst-core` to skip mbedTLS for faster builds (this also disables
encryption — only do this for testing).

## Send your first packet

```rust
use tst_srt::SocketBuilder;
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut socket = SocketBuilder::new()
        .latency(Duration::from_millis(120))
        .connect("127.0.0.1:9000")?;
    socket.send(b"hello, srt")?;
    socket.close()?;
    Ok(())
}
```

Run with `cargo run`. The 120 ms latency is the conventional starting
point for live SRT — both peers must agree on it. `connect` blocks
until the SRT handshake completes; `send` blocks until the message is
queued for the wire.

## Receive your first packet

```rust
use tst_srt::ListenerBuilder;
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut listener = ListenerBuilder::new()
        .latency(Duration::from_millis(120))
        .bind("0.0.0.0:9000")?;
    let (mut socket, peer) = listener.accept()?;
    println!("accepted from {peer}");
    let mut buf = [0u8; 1500];
    loop {
        match socket.recv(&mut buf) {
            Ok(n) => println!("recv {n} bytes: {:?}", &buf[..n.min(20)]),
            Err(tst_srt::error::RecvError::ConnectionBroken) => break,
            Err(e) => return Err(Box::new(e)),
        }
    }
    Ok(())
}
```

Run the receiver in one terminal, then the sender from the previous
section in another. The receiver prints
`recv 10 bytes: [104, 101, 108, 108, 111, ...]` and exits cleanly when
the peer closes. The 1500-byte buffer is comfortably above the default
SRT payload size (1316 bytes), so each `recv` returns one whole
message.

## Send a video frame

Switch from raw bytes to a typed video sender:

```rust
use tst_core::mpegts::mux::Config;
use tst_pipeline::MuxSender;
use tst_srt::{SocketBuilder, SrtTransport};
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let socket = SocketBuilder::new()
        .latency(Duration::from_millis(120))
        .connect("127.0.0.1:9000")?;
    let transport = SrtTransport::new(socket);
    let sender = MuxSender::new(Config::default(), transport)?;

    // Synthetic Annex-B IDR access unit + KLV blob.
    let nal = vec![0x00, 0x00, 0x00, 0x01, 0x65, /* ... payload bytes ... */];
    let klv = vec![0x06, 0x0E, 0x2B, 0x34, /* ... ST 0601 record ... */];
    sender.send_video(&nal, /*pts_90khz=*/ 0, /*key_frame=*/ true)?;
    // metadata_service_id = 0x00 is the default per ST 1402.2 App. B Table 2.
    sender.send_klv(&klv, /*pts_90khz=*/ 0, /*metadata_service_id=*/ 0x00)?;

    sender.close();
    Ok(())
}
```

`MuxSender` wraps an `mpegts::mux::Muxer` and an `SrtTransport`. It
auto-mux's NAL units and KLV blobs into a single MPEG-TS stream and
sends each TS chunk over SRT. `pts_90khz` is in 90 kHz ticks (the TS
clock); `key_frame` should be true for IDR frames.

In production, replace the synthetic generator with your encoder's
output. See
[../crates/tst-srt/examples/pipeline_send_to_socket.rs](../crates/tst-srt/examples/pipeline_send_to_socket.rs)
for a runnable version with five frames and pacing.

## Run the example pair

The fastest way to see end-to-end behavior is the bundled example
pair:

```bash
# terminal A
cargo run --example srt_listener_to_file -- 127.0.0.1:9000 /tmp/out.ts
# terminal B
cargo run --example pipeline_send_to_socket -- 127.0.0.1:9000
```

The receiver writes incoming bytes to `/tmp/out.ts`. After the sender
exits, `file /tmp/out.ts` reports `MPEG transport stream data`. The
sender produces five synthetic frames plus matching KLV records at
roughly 30 fps, then closes; the receiver drains until the connection
is broken and exits cleanly.

## Where to go next

- [architecture.md](architecture.md) — how the pieces fit together.
- [guide-srt.md](guide-srt.md) — `Socket`, `Listener`, encryption,
  latency, stats.
- [guide-klv.md](guide-klv.md) — encoding and decoding ST 0601 KLV.
- [guide-mpegts-mux.md](guide-mpegts-mux.md) — the TS muxer's knobs.
- [guide-pipeline.md](guide-pipeline.md) — picking among `MuxSender`,
  `Sender`, and `RawSender`.
- [cookbook.md](cookbook.md) — recipes for common multi-step tasks.
- [troubleshooting.md](troubleshooting.md) — common failure modes.
- [compatibility.md](compatibility.md) — feature-by-feature support
  matrix.
