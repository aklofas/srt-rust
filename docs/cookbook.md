# Cookbook

Common multi-step recipes. Each recipe is a short narrative + a code block + a link to the corresponding runnable example. Run any example with `cargo run --example <name>`. The full set of examples lives at `crates/tst-srt/examples/`.

## Recipes

### 1. Send video + KLV with passphrase encryption

Reach for this when you need a secure uplink. SRT's encryption is AES-CTR with a passphrase-derived key, negotiated during the handshake; both peers must agree on the same passphrase and key length.

The diff against an unencrypted setup is small: `passphrase(...)` plus `key_length(...)` on both the `SocketBuilder` and the `ListenerBuilder`. `Passphrase::new` validates length (10–79 ASCII-printable bytes, libsrt's own constraint).

```rust,no_run
use tst_srt::{KeyLength, Passphrase, SocketBuilder};
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let passphrase = Passphrase::new("shared-secret-not-for-production")?;
    let mut socket = SocketBuilder::new()
        .passphrase(passphrase)
        .key_length(KeyLength::Aes256)
        .latency(Duration::from_millis(120))
        .connect("127.0.0.1:9000")?;
    socket.send(b"encrypted hello")?;
    socket.close()?;
    Ok(())
}
```

Runnable: [../crates/tst-srt/examples/encrypted_send_recv.rs](../crates/tst-srt/examples/encrypted_send_recv.rs).

### 2. Survive a flaky transport with reconnect + gap buffer

Reach for this when the wire is lossy — radio links, NAT timeouts, listener restarts. `ManagedTransport<T>` decorates any `Transport` impl with a reconnect loop and a bounded gap buffer; the wrapped sender shell sees a `Transport` that occasionally pauses but never fails on transient breakage.

The factory closure rebuilds the inner transport on demand. `ReconnectPolicy` controls retries, backoff, and gap-buffer overflow behaviour.

```rust,no_run
use tst_core::mpegts::mux::Config;
use tst_pipeline::{
    BackoffStrategy, ManagedTransport, MuxSender, OverflowPolicy, ReconnectPolicy, TransportError,
};
use tst_srt::{SocketBuilder, SrtTransport};
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let factory = || -> Result<SrtTransport, TransportError> {
        let socket = SocketBuilder::new()
            .latency(Duration::from_millis(120))
            .connect("127.0.0.1:9000")
            .map_err(|e| TransportError::Broken(format!("connect failed: {e}")))?;
        Ok(SrtTransport::new(socket))
    };
    let initial = factory()?;
    let policy = ReconnectPolicy {
        max_attempts: Some(20),
        backoff: BackoffStrategy::Exponential {
            base: Duration::from_millis(100),
            max: Duration::from_secs(10),
        },
        gap_buffer_capacity: 256,
        overflow_policy: OverflowPolicy::DropOldest,
    };
    let managed = ManagedTransport::new(initial, factory, policy);
    let _sender = MuxSender::new(Config::default(), managed)?;
    Ok(())
}
```

Runnable: [../crates/tst-srt/examples/managed_reconnect.rs](../crates/tst-srt/examples/managed_reconnect.rs).

### 3. Mux to a file (no SRT, no transport)

Reach for this when you want the muxer's output without any networking — building test fixtures, validating output against TSDuck/ffprobe, or running an offline pipeline. `Muxer` is the standalone TS muxer; `push_video` and `push_klv` queue input, `pull` drains 188-byte-aligned TS packets into a caller-provided buffer.

The drain loop is the standard pattern: push input, then pull until `pull` returns 0. Drain after every push so muxer memory stays bounded.

```rust,no_run
use tst_core::mpegts::mux::{Config, Muxer};
use std::fs::File;
use std::io::Write;

fn main() -> std::io::Result<()> {
    let mut mux = Muxer::new(Config::default()).expect("valid config");
    let mut out = File::create("out.ts")?;
    let mut buf = [0u8; 1316];
    for i in 0..150i64 {
        let pts = i * 3000; // 30 fps on 90 kHz clock
        let nal = vec![0x00, 0x00, 0x00, 0x01, 0x65, 0xAA];
        let klv = vec![0x06, 0x0E, 0x2B, 0x34, /* ... */];
        mux.push_video(&nal, pts, i == 0).expect("push_video");
        // metadata_service_id=0x00 is the ST 1402.2 App. B Table 2 default;
        // override to mirror a non-default metadata_klva(svc) PMT descriptor.
        mux.push_klv(&klv, pts, /*metadata_service_id=*/ 0x00).expect("push_klv");
        loop {
            let n = mux.pull(&mut buf);
            if n == 0 { break; }
            out.write_all(&buf[..n])?;
        }
    }
    Ok(())
}
```

Runnable: [../crates/tst-srt/examples/mux_to_file.rs](../crates/tst-srt/examples/mux_to_file.rs).

### 4. Relay a captured `.ts` file over SRT

Reach for this when you have a `.ts` capture you want to replay over SRT — regression-testing receivers, rebroadcasting an archive, exercising a downstream pipeline. `Sender` accepts arbitrary byte chunks, verifies TS sync, and emits 7-packet (1316-byte) bundles to the wrapped transport.

The sender is byte-stream oriented — file reads of any size are fine, the sender handles 188-alignment and bundling internally. `flush()` emits any buffered partial bundle so the tail of a finite input reaches the wire.

```rust,no_run
use tst_pipeline::{Sender, SenderConfig};
use tst_srt::{SocketBuilder, SrtTransport};
use std::fs::File;
use std::io::Read;
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let socket = SocketBuilder::new()
        .latency(Duration::from_millis(120))
        .connect("127.0.0.1:9000")?;
    let mut sender = Sender::new(SrtTransport::new(socket), SenderConfig::default());
    let mut file = File::open("input.ts")?;
    let mut buf = vec![0u8; 4096];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 { break; }
        sender.send_ts(&buf[..n])?;
    }
    sender.flush()?;
    sender.close();
    Ok(())
}
```

Runnable: [../crates/tst-srt/examples/ts_relay_from_file.rs](../crates/tst-srt/examples/ts_relay_from_file.rs).

### 5. Receive into a file

Reach for this when archiving a stream or building a test fixture from a live producer. `Listener::accept` returns a connected `Socket`; the recv loop drains until `ConnectionBroken`.

A 1500-byte buffer comfortably fits SRT's default 1316-byte payload, so each `recv` returns one whole message. The three-arm match handles data, clean close, and defensive timeout.

```rust,no_run
use tst_srt::ListenerBuilder;
use std::fs::File;
use std::io::Write;
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut listener = ListenerBuilder::new()
        .latency(Duration::from_millis(120))
        .bind("0.0.0.0:9000")?;
    let (mut socket, _peer) = listener.accept()?;
    let mut out = File::create("out.ts")?;
    let mut buf = [0u8; 1500];
    loop {
        match socket.recv(&mut buf) {
            Ok(n) => out.write_all(&buf[..n])?,
            Err(tst_srt::error::RecvError::ConnectionBroken) => break,
            Err(tst_srt::error::RecvError::TimedOut) => continue,
            Err(e) => return Err(Box::new(e)),
        }
    }
    Ok(())
}
```

Runnable: [../crates/tst-srt/examples/srt_listener_to_file.rs](../crates/tst-srt/examples/srt_listener_to_file.rs).

### 6. Decode ST 0601 from a captured `.klv` blob

Reach for this when validating producer output, building dashboards on top of captured data, or debugging a receiver. The two-step pipeline is: extract KLV blobs from the `.ts` first, then decode each blob through the strictness ladder.

`extract_klv` parses PAT and PMT to find the KLV PID (registration descriptor `KLVA`), demuxes PES packets on that PID, and writes each PES payload as `<prefix>_NNNN.klv` (0-indexed via `enumerate()`). Each `.klv` blob then feeds `klv_decode_file`, which walks the ladder `decode_strict_compliance` → `decode_strict` → `decode` → `decode_unchecked`, reporting which level accepted.

```rust,no_run
use tst_core::klv::st0601::{decode, decode_strict, decode_strict_compliance};
use std::fs;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let buf = fs::read("capture_0000.klv")?;
    let parsed = decode_strict_compliance(&buf)
        .or_else(|_| decode_strict(&buf))
        .or_else(|_| decode(&buf))?;
    if let Some(ts) = parsed.timestamp_us {
        println!("timestamp_us: {ts}");
    }
    if let (Some(lat), Some(lon)) = (parsed.sensor_lat_deg, parsed.sensor_lon_deg) {
        println!("sensor: {lat:.6}, {lon:.6}");
    }
    Ok(())
}
```

Runnable: [../crates/tst-srt/examples/extract_klv.rs](../crates/tst-srt/examples/extract_klv.rs) and [../crates/tst-srt/examples/klv_decode_file.rs](../crates/tst-srt/examples/klv_decode_file.rs).

### 7. Encode ST 0601 from typed values

Reach for this when synthesizing KLV for tests, generating fixtures, or translating from a different metadata format in a gateway. Every field on `UasDatalinkLs` is `Option<T>` — set `Some(...)` on the fields you want emitted, leave the rest as `None`.

`encode_to_vec` auto-emits Tag 1 (16-bit BCC checksum, mandated last) and Tag 65 (UAS LS Version Number, mandated present) when the caller didn't set them. So a default-constructed record with a few typed fields produces wire bytes that satisfy strict-compliance validation out of the box.

```rust,no_run
use tst_core::klv::st0601::{UasDatalinkLs, encode_to_vec};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut rec = UasDatalinkLs::default();
    rec.timestamp_us = Some(1_700_000_000_000_000);
    rec.platform_designation = Some("test-platform".into());
    rec.sensor_lat_deg = Some(33.6800);
    rec.sensor_lon_deg = Some(-118.5500);
    rec.sensor_alt_m = Some(3500.0);
    rec.platform_heading_deg = Some(217.456);
    rec.platform_pitch_deg = Some(-2.150);
    rec.platform_roll_deg = Some(-1.875);
    let encoded = encode_to_vec(&rec)?;
    println!("encoded {} bytes", encoded.len());
    Ok(())
}
```

Runnable: [../crates/tst-srt/examples/klv_encode_minimal.rs](../crates/tst-srt/examples/klv_encode_minimal.rs).

### 8. Use a custom (non-SRT) transport

Reach for this when the sender shells fit but the wire isn't SRT — UDP, file, in-memory test harness, your own protocol. `MuxSender`, `Sender`, and `RawSender` are all generic over `T: Transport`; implement the trait once and they all compose.

The trait is four methods: `send_bytes`, `max_payload`, `is_alive`, `close`. Your impl needs to be `Send`, not `Sync` — the shells handle internal synchronization where required.

```rust,no_run
use tst_pipeline::{Transport, TransportError};
use std::sync::{Arc, Mutex};

struct MemTransport {
    packets: Arc<Mutex<Vec<Vec<u8>>>>,
    alive: bool,
    max_payload: usize,
}

impl Transport for MemTransport {
    fn send_bytes(&mut self, msg: &[u8]) -> Result<(), TransportError> {
        if msg.len() > self.max_payload {
            return Err(TransportError::TooLarge { len: msg.len(), max: self.max_payload });
        }
        if !self.alive { return Err(TransportError::Closed); }
        self.packets.lock().unwrap().push(msg.to_vec());
        Ok(())
    }
    fn max_payload(&self) -> usize { self.max_payload }
    fn is_alive(&self) -> bool { self.alive }
    fn close(&mut self) { self.alive = false; }
}
```

Runnable: [../crates/tst-srt/examples/custom_transport.rs](../crates/tst-srt/examples/custom_transport.rs).

### 9. Mux H.265 + sync KLV

Reach for this when the encoder produces HEVC, or when the receiver requires strict ST 1402 sync metadata (PMT stream_type 0x15) instead of the default async private-data shape. Three knobs flip on `Config`: codec → `H265`, KLV stream type → `SynchronousMetadata`, `carries_pts` → `true`.

**Sync KLV auto-wraps in the muxer.** When you configure `KlvStreamType::SynchronousMetadata`, `Muxer::push_klv` auto-prepends a 5-byte `Metadata_AU_cell` header per ITU-T H.222.0 V9 § 2.12.4.2 (Tables 2-155+2-156) before TS-framing. Pass raw KLV LS bytes — do not pre-wrap. PTS lives in the PES header (per § 2.12.4.1). See [guide-mpegts-mux.md](guide-mpegts-mux.md) §"KLV-in-TS modes".

```rust,no_run
use tst_core::mpegts::mux::{Config, KlvStreamType, Muxer, VideoCodec};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cfg = Config::builder()
        .add_program(1, 0x1000)
        .add_video(0x1011, VideoCodec::H265)
        .add_klv(0x1031, KlvStreamType::SynchronousMetadata, /*carries_pts=*/ true)
        .end_program()
        .build()?;
    let mut mux = Muxer::new(cfg)?;
    let inner_klv: Vec<u8> = vec![/* ST 0601 bytes */];
    // Muxer auto-prepends the 5-byte AU cell header. metadata_service_id
    // defaults to 0x00 per ST 1402.2 App. B Table 2.
    mux.push_klv(&inner_klv, /*pts_90khz=*/ 0, /*metadata_service_id=*/ 0x00)?;
    Ok(())
}
```

Runnable: [../crates/tst-srt/examples/mux_h265_with_klv.rs](../crates/tst-srt/examples/mux_h265_with_klv.rs).

### 10. Print live `Stats` from a sender

Reach for this when building an operational dashboard, instrumenting a sender for production telemetry, or debugging packet loss in the field. `Socket::stats()` returns a snapshot of libsrt's per-socket counters — call it periodically and surface the deltas.

The most operationally interesting fields on a sender: `bytes_sent`, `packets_lost_send_side`, `packets_retransmitted`, `rtt`, and `mbps_estimated_bandwidth`. (Loss/drop counters are split by which side observed them — read `*_send_side` on a sender, `*_recv_side` on a receiver.) There's no standalone example for this; see [guide-srt.md](guide-srt.md) §`Stats` for the full field list and [../crates/tst-srt/examples/managed_reconnect.rs](../crates/tst-srt/examples/managed_reconnect.rs) for similar peer-thread observation patterns.

```rust,no_run
use tst_srt::SocketBuilder;
use std::thread;
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let socket = SocketBuilder::new()
        .latency(Duration::from_millis(120))
        .connect("127.0.0.1:9000")?;
    for _ in 0..10 {
        thread::sleep(Duration::from_secs(1));
        let s = socket.stats()?;
        println!(
            "bytes_sent={} packets_lost_send_side={} retrans={} rtt={:?} bw_mbps={:.2}",
            s.bytes_sent, s.packets_lost_send_side, s.packets_retransmitted,
            s.rtt, s.mbps_estimated_bandwidth,
        );
    }
    Ok(())
}
```

No standalone example; see [../crates/tst-srt/examples/managed_reconnect.rs](../crates/tst-srt/examples/managed_reconnect.rs) and [guide-srt.md](guide-srt.md) §`Stats`.

### 11. Open a sender from an `srt://...?...` URL

Useful when the connection target and tuning live in deployment config
files (or are passed in by an orchestrator). Build a `SocketConfig`
from the parsed URL's overlay, then connect.

```rust,no_run
use tst_srt::{SocketBuilder, SrtUrl};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let parsed = SrtUrl::parse(
        "srt://camera.local:9000?streamid=front&latency=200&passphrase=hunter-too-long",
    )?;
    let mut config = SocketBuilder::new().config();
    parsed.overlay.apply_to_socket(&mut config);
    let _socket = tst_srt::Socket::connect_with(
        &config,
        format!("{}:{}", parsed.host, parsed.port).as_str(),
    )?;
    Ok(())
}
```

Runnable: [../crates/tst-srt/examples/sender_from_url.rs](../crates/tst-srt/examples/sender_from_url.rs).

### 12. Pair sync-KLV with video AUs by nearest PTS

Reach for this when an encoder emits sync-KLV (PMT stream_type 0x15, H.222.0 § 2.12.4.2 `Metadata_AU_cell`) synchronized to video frames (one KLV per frame, KLV PES PTS = frame PTS) and you want to consume frame + telemetry as a paired record. By design, `mpegts::demux` does NOT pair sync-KLV with video AUs — it surfaces them as independent stream-tagged events with full timing info, and the pairing tolerance is consumer-domain knowledge. This recipe is the canonical nearest-PTS pattern.

Match BOTH `MetadataKind::KlvSyncAuCell` AND `MetadataKind::KlvAsync`. The natural intuition is "sync KLV is the kind that needs pairing," but many production ISR encoders declare a PID `stream_type=0x15` and ship bare KLV without the 5-byte AU cell header. The demuxer surfaces those bytes as `KlvAsync` with the PES PTS preserved on the parent event. That `KlvAsync` is still PTS-aligned with video; matching only `KlvSyncAuCell` silently drops the most common shape we see in the field.

```rust,no_run
use tst_core::mpegts::demux::{DemuxEvent, Demuxer, MetadataKind, SamplePayload};
use std::collections::VecDeque;
use std::fs;

// 0.3 s at 90 kHz — wide enough to absorb encoder timestamp drift,
// narrow enough to reject a coincidental near-match from the next GOP.
const PAIRING_TOLERANCE_TICKS: i64 = 3 * 9_000;
// 32 entries of KLV history. ~1 s at 30 fps + 1 KLV/frame; 32 s at 1 Hz KLV.
const KLV_HISTORY_LEN: usize = 32;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let bytes = fs::read("input.ts")?;
    let mut d = Demuxer::new();
    d.feed(&bytes)?;
    d.flush();
    let mut history: VecDeque<(i64, Vec<u8>)> = VecDeque::with_capacity(KLV_HISTORY_LEN);
    let (mut paired, mut unpaired) = (0usize, 0usize);
    while let Some(e) = d.next_event() {
        match e {
            DemuxEvent::Metadata {
                pts,
                kind: MetadataKind::KlvSyncAuCell | MetadataKind::KlvAsync,
                payload,
                ..
            } => {
                history.push_back((pts, payload));
                if history.len() > KLV_HISTORY_LEN {
                    history.pop_front();
                }
            }
            DemuxEvent::Sample {
                pts,
                payload: SamplePayload::Video { .. },
                ..
            } => {
                let nearest = history.iter().min_by_key(|(kpts, _)| (kpts - pts).abs());
                match nearest {
                    Some((kpts, _)) if (kpts - pts).abs() <= PAIRING_TOLERANCE_TICKS => {
                        paired += 1;
                    }
                    _ => unpaired += 1,
                }
            }
            _ => {}
        }
    }
    println!("paired={paired} unpaired={unpaired}");
    Ok(())
}
```

Tolerance is consumer-domain knowledge. Most encoders emit KLV PES PTS exactly equal to frame PTS; a window of a few hundred milliseconds covers minor encoder drift. See [examples/pair_sync_klv.rs](../crates/tst-srt/examples/pair_sync_klv.rs) for the full runnable form.

Runnable: [../crates/tst-srt/examples/pair_sync_klv.rs](../crates/tst-srt/examples/pair_sync_klv.rs); see also [../crates/tst-srt/examples/demux_to_events.rs](../crates/tst-srt/examples/demux_to_events.rs) for the file-feed shape.

### 13. Sample-and-hold async-KLV against video frames

Reach for this when KLV is emitted independently of video — typically 1–10 Hz async metadata against 25–60 fps video. The canonical pairing is "the most recent KLV record where `klv.pts <= frame.pts`." There is no ambiguity about which KLV pairs with which frame; the only knob is whether to drop a frame when the most recent KLV is too stale.

```rust,no_run
use tst_core::mpegts::demux::{DemuxEvent, Demuxer, MetadataKind, SamplePayload};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut d = Demuxer::new();
    // Maintain "current KLV state" per metadata PID:
    let mut last_klv: Option<(i64, Vec<u8>)> = None;
    while let Some(e) = d.next_event() {
        match e {
            DemuxEvent::Metadata { pts, kind: MetadataKind::KlvAsync, payload, .. } => {
                last_klv = Some((pts, payload));
            }
            DemuxEvent::Sample { payload: SamplePayload::Video { .. }, pts: _frame_pts, .. } => {
                // Use last_klv if available, regardless of how stale.
                // Optional: compare ages and drop if stale beyond a freshness window.
                let _telemetry = last_klv.as_ref().map(|(_, payload)| payload);
            }
            _ => {}
        }
    }
    Ok(())
}
```

Runnable: see [../crates/tst-srt/examples/demux_to_events.rs](../crates/tst-srt/examples/demux_to_events.rs) for the file-feed shape; [../crates/tst-srt/examples/pair_sync_klv.rs](../crates/tst-srt/examples/pair_sync_klv.rs) is the related nearest-PTS sibling for sync KLV.

### 14. EO + IR sensor pair with shared async-KLV

Reach for this when the platform carries two sensors (visible + thermal) and one async metadata stream serves both. Both video streams attach the same KLV state; there is no per-stream pairing logic. The demuxer surfaces the topology as a `ProgramMap` with two `StreamInfo` rows of `StreamKind::Video(_)` and one `StreamKind::KlvAsync`; the `klv_links` table reports the encoder-declared (or inferred / overridden) linkage.

```rust,no_run
use tst_core::mpegts::demux::{DemuxEvent, Demuxer, MetadataKind, SamplePayload};

fn process_eo(_pts: i64, _klv: Option<&[u8]>) {}
fn process_ir(_pts: i64, _klv: Option<&[u8]>) {}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut d = Demuxer::new();
    let mut last_klv: Option<Vec<u8>> = None;
    while let Some(e) = d.next_event() {
        match e {
            DemuxEvent::Metadata { kind: MetadataKind::KlvAsync, payload, .. } => {
                last_klv = Some(payload);
            }
            DemuxEvent::Sample { stream, payload: SamplePayload::Video { .. }, pts, .. } => {
                match stream.pid {
                    0x100 => process_eo(pts, last_klv.as_deref()),
                    0x101 => process_ir(pts, last_klv.as_deref()),
                    _ => {}
                }
            }
            _ => {}
        }
    }
    Ok(())
}
```

If the encoder declares the linkage via `metadata_descriptor`, the demuxer surfaces it as `KlvLink { source: LinkSource::Declared, .. }` in `ProgramMap.klv_links`. Use it as a hint when assigning routes; trust your `treat_as` overrides if you know the encoder lies.

Runnable: see [../crates/tst-srt/examples/demux_to_events.rs](../crates/tst-srt/examples/demux_to_events.rs) for the file-feed shape; [../crates/tst-srt/examples/pair_sync_klv.rs](../crates/tst-srt/examples/pair_sync_klv.rs) is the related sync-KLV sibling.

### 15. Label EO + IR + KLV streams in a multi-stream program

Multi-stream programs (`mpegts::mux` Path 3) carry several PIDs in one
program. Per-stream PMT descriptors let receivers (TSDuck, ffprobe, our
own `Demuxer`) render which PID is which without external configuration.

```rust,no_run
use tst_core::mpegts::descriptors as desc;
use tst_core::mpegts::mux::{Config, KlvStreamType, Muxer, VideoCodec};

const EO_PID: u16 = 0x0100;
const IR_PID: u16 = 0x0101;
const KLV_PID: u16 = 0x0102;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cfg = Config::builder()
        .add_video(EO_PID, VideoCodec::H264)
        .stream_descriptors_for_video(0, vec![desc::user_private(b"EO 1080p")])
        .add_video(IR_PID, VideoCodec::H264)
        .stream_descriptors_for_video(1, vec![desc::user_private(b"IR 640x480")])
        .add_klv(KLV_PID, KlvStreamType::SynchronousMetadata, true)
        .stream_descriptors_for_klv(0, vec![
            // 0x26 + 0x27 are the canonical pair for stream_type=0x15 KLV
            // (the muxer's auto-emitted KLVA Registration only fires for
            // PrivateData KLV, not SynchronousMetadata).
            desc::metadata_klva(0x00),
            desc::metadata_std(0, 0, 0),
            // Plus a human label.
            desc::user_private(b"KLV_SYNC"),
        ])
        .build()?;

    let mut _mux = Muxer::new(cfg)?;
    // ...push frames as usual...
    Ok(())
}
```

Validate the labels show up on the receiving end:

```bash
tstables --pid <pmt-pid> output.ts | grep -A1 "Forbidden Descriptor"
```

Or in Rust on the receive side, decode `StreamInfo::raw_descriptors`
directly (see `guide-mpegts-demux.md` "Reading per-stream descriptors").

Runnable example: `cargo run --example mux_dual_camera`.

### 16. Repack two single-program inputs into one multi-program TS

When you have two independent (EO + IR + KLV) feeds and need to ship them
through one SRT socket without forcing each to its own UDP port:

```rust,no_run
use tst_core::mpegts::mux::{Config, KlvStreamType, VideoCodec};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::builder()
        .add_program(1, 0x1000)
            .add_video(0x1011, VideoCodec::H264)
            .add_klv(0x1031, KlvStreamType::PrivateData, false)
            .end_program()
        .add_program(2, 0x1100)
            .add_video(0x1111, VideoCodec::H264)
            .add_klv(0x1131, KlvStreamType::PrivateData, false)
            .end_program()
        .build()?;

    // Resolve handles per-program; push_video_to/push_klv_to route to
    // the correct elementary stream even when two programs carry the same
    // codec.  The bare push_video / push_klv reject with AmbiguousTarget
    // when more than one stream of that kind exists across all programs.
    // let mux = Muxer::new(config)?;
    // let [v1] = mux.video_handles_for_program(1)[..] else { ... };
    // let [v2] = mux.video_handles_for_program(2)[..] else { ... };
    // mux.push_video_to(v1, pts, dts, is_keyframe, &nal_bytes)?;
    Ok(())
}
```

On the receive side, the consumer sees two independent `ProgramMap` events
and can route `Sample`/`Metadata` events by `stream.program_number`. The
receiver picks one program of interest with ffmpeg `-map p:N` or TSDuck
`--pid-only`. PID uniqueness across programs is required by the muxer;
renumber program 2's input PIDs into a non-conflicting range during the
demux→remux step.

Runnable: [../crates/tst-srt/examples/repack_two_programs.rs](../crates/tst-srt/examples/repack_two_programs.rs).

### 17. Extract video resolution and profile from a demuxed stream

Reach for this when you need typed codec information (width, height, profile,
level, frame rate, color) and are already demuxing the stream. The demuxer
surfaces raw NAL bytes; you call the matching `codec::*` parser explicitly
on each `Sample` event. `parse_parameter_sets` is safe to call on every
sample — it skips non-SPS/PPS NALs silently and returns `Ok` with empty
maps on P-frames.

```rust,no_run
use tst_core::codec::h264;
use tst_core::mpegts::demux::{DemuxEvent, Demuxer, SamplePayload, VideoCodec, VideoPayload};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut dx = Demuxer::new();
    // ... feed bytes to dx ...
    while let Some(ev) = dx.next_event() {
        if let DemuxEvent::Sample {
            payload: SamplePayload::Video { codec: VideoCodec::H264, payload: VideoPayload::Nals(ref nals) },
            ..
        } = ev
        {
            if let Ok(ps) = h264::parse_parameter_sets(nals) {
                if let Some(sps) = ps.sps_by_id.values().next() {
                    println!(
                        "{}x{} profile={} level={}",
                        sps.width, sps.height, sps.profile_idc, sps.level_idc
                    );
                }
            }
        }
    }
    Ok(())
}
```

For H.265 substitute `h265::parse_parameter_sets` and use
`sps.general_profile_idc` / `sps.general_level_idc` (level is `× 30` — level
4.0 is stored as 120). The pattern is identical; only the import and field
names differ.

Runnable: [../crates/tst-srt/examples/parse_video_parameters.rs](../crates/tst-srt/examples/parse_video_parameters.rs) — shows change-driven logging per PID across H.264 and H.265 in one pass.

### 18. Reconstitute Annex B parameter sets for decoder replay

Reach for this when you need to hand SPS / PPS bytes to a hardware decoder,
encoder re-init, or a library that expects Annex-B-framed codec configuration.
The `raw_rbsp` field on each parsed struct preserves the input bytes verbatim
(including emulation-prevention bytes) exactly as received from the demuxer.
Prepend a 4-byte start code to get conformant Annex B framing:

```rust,no_run
use tst_core::codec::h264;
use tst_core::mpegts::demux::{DemuxEvent, Demuxer, SamplePayload, VideoCodec, VideoPayload};

fn to_annex_b(rbsp: &[u8]) -> Vec<u8> {
    // Same for H.264 and H.265 — the demuxer includes the NAL header byte(s)
    // in the payload field, so raw_rbsp already contains the full NAL unit
    // minus its Annex-B start code. Just prepend the start code.
    let mut out = vec![0x00, 0x00, 0x00, 0x01];
    out.extend_from_slice(rbsp);
    out
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut dx = Demuxer::new();
    // ... feed bytes ...
    while let Some(ev) = dx.next_event() {
        if let DemuxEvent::Sample {
            payload: SamplePayload::Video { codec: VideoCodec::H264, payload: VideoPayload::Nals(ref nals) },
            ..
        } = ev
        {
            if let Ok(ps) = h264::parse_parameter_sets(nals) {
                let mut decoder_config: Vec<u8> = Vec::new();
                for sps in ps.sps_by_id.values() {
                    decoder_config.extend(to_annex_b(&sps.raw_rbsp));
                }
                for pps in ps.pps_by_id.values() {
                    decoder_config.extend(to_annex_b(&pps.raw_rbsp));
                }
                // Pass decoder_config to your hardware decoder or codec library.
            }
        }
    }
    Ok(())
}
```

Runnable: [../crates/tst-srt/examples/parse_video_parameters.rs](../crates/tst-srt/examples/parse_video_parameters.rs) shows the full demux-to-parse loop; see `docs/guide-codec.md` for the decoder-replay section.

### 19. Mux audio + video + KLV in a single program

Build a three-stream program where audio PTS-aligns with video for
synchronized playback, and KLV records emit on the same PCR clock.

```rust
use tst_core::mpegts::mux::{
    AudioCodec, ConfigBuilder, KlvStreamType, Muxer, VideoCodec,
};

let cfg = ConfigBuilder::new()
    .add_program(1, 0x1000)
    .add_video(0x100, VideoCodec::H264)
    .add_klv(0x200, KlvStreamType::PrivateData, /*carries_pts=*/ false)
    // add_audio_with_language auto-emits an iso_639_language_descriptor
    // (tag 0x0A) on the PMT entry — receivers (browsers, transcoders,
    // players) get a language hint without manually wiring descriptors.
    // Use plain add_audio(pid, codec) when language is unknown / unset.
    .add_audio_with_language(0x300, AudioCodec::Aac, *b"eng")
    .end_program()
    .build()?;

let mut muxer = Muxer::new(cfg)?;

for frame_idx in 0..30 {
    let pts = 90_000 + frame_idx * 3000;
    muxer.push_video(&video_au_bytes, pts, /*key_frame=*/ frame_idx % 30 == 0)?;
    muxer.push_audio(&aac_frame_bytes, pts)?;
    if frame_idx % 30 == 0 {
        muxer.push_klv(&klv_record, pts, /*metadata_service_id=*/ 0x00)?;
    }
    // Drain to your transport.
}
```

Full example: [`../crates/tst-srt/examples/mux_audio_video_klv.rs`](../crates/tst-srt/examples/mux_audio_video_klv.rs).

### 20. Inject WebVTT POI cues into a live MPEG-TS uplink

Use case: a sensor / orchestrator wants to mark Points of Interest
in a live SRT/TS stream so the downstream HLS player (hls.js etc.)
can render them as captions.

```rust
use tst_core::mpegts::mux::{Config, Muxer, SubtitleCodec, VideoCodec};

let cfg = Config::builder()
    .add_program(1, 0x100)
        .add_video(0x101, VideoCodec::H264)
        .add_subtitle(0x200, SubtitleCodec::WebVttInTs)
    .end_program()
    .build()?;
let mut mux = Muxer::new(cfg)?;
let h = mux.subtitle_handles()[0];

// Each POI: assemble a WebVTT cue and push at the wall-clock PTS.
let cue = "WEBVTT\n\n00:00:01.000 --> 00:00:05.000\nPOI: target acquired\n";
mux.push_subtitle_to(h, 90_000, cue.as_bytes())?;
// Drain TS bytes via `mux.pull(&mut buf)` in a loop until it returns
// 0 (queue empty); see the runnable example for a `drain_all` helper.
```

Runnable: `cargo run --example mux_with_webvtt_subtitles -- output.ts`.

### 21. Extract subtitle PES bytes from a captured `.ts` file

Use case: receive-side inspection — what subtitle codecs are in a
capture, and what's the cue text?

```rust
use tst_core::mpegts::demux::{DemuxEvent, Demuxer, SamplePayload};

let mut demux = Demuxer::new();
demux.feed(&bytes)?;
demux.flush();
while let Some(e) = demux.next_event() {
    if let DemuxEvent::Sample {
        stream,
        payload: SamplePayload::Subtitle { codec, payload },
        ..
    } = e
    {
        println!(
            "PID 0x{:04x} codec={:?} bytes={}",
            stream.pid,
            codec,
            payload.len()
        );
    }
}
```

Runnable: `cargo run --example demux_subtitle_file -- input.ts`.

### 22. Streaming H.266 / VVC video with synchronous KLV metadata

H.266 (VVC) carries in MPEG-TS under PMT `stream_type = 0x33` per the
ITU-T H.222.0 amendment for VVC; the muxer emits that byte automatically
when `VideoCodec::H266` is configured. The push contract is identical to
H.264 / H.265 — Annex-B framing on `push_video`, one PES per call. Only
the codec flag and the SPS / PPS / VPS bytes change.

The recipe below mirrors recipe 9 (H.265 + sync KLV) — flip the codec to
`VideoCodec::H266` and feed H.266 NAL bytes (NAL types 14 / 15 / 16 for
VPS / SPS / PPS).

```rust,no_run
use tst_core::mpegts::mux::{Config, KlvStreamType, Muxer, VideoCodec};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cfg = Config::builder()
        .add_program(1, 0x1000)
        .add_video(0x1011, VideoCodec::H266)
        .add_klv(0x1031, KlvStreamType::SynchronousMetadata, /*carries_pts=*/ true)
        .end_program()
        .build()?;
    let mut mux = Muxer::new(cfg)?;
    let inner_klv: Vec<u8> = vec![/* ST 0601 bytes */];
    // Muxer auto-prepends the 5-byte H.222.0 § 2.12.4.2 AU cell header.
    // metadata_service_id defaults to 0x00 per ST 1402.2 App. B Table 2.
    mux.push_klv(&inner_klv, /*pts_90khz=*/ 0, /*metadata_service_id=*/ 0x00)?;
    Ok(())
}
```

Runnable: [../crates/tst-srt/examples/mux_h266_with_klv.rs](../crates/tst-srt/examples/mux_h266_with_klv.rs).

### 23. Streaming AV1 video with KLV metadata

AV1 uses OBU framing — fundamentally different from the NAL-shaped codecs
(H.264 / H.265 / H.266). Key differences when feeding `Muxer::push_video`:

- **No Annex-B start codes.** OBUs are self-describing and length-prefixed
  via LEB128. Concatenating OBUs with no separator produces a complete
  access unit.
- **AV1-in-MPEG-2-TS binding §3.1 requires `obu_has_size_field = 1`** on
  every OBU so the demultiplexer can walk the OBU stream without a
  separate framing layer.
- **PMT `stream_type = 0x06`** plus an auto-emitted `AV01`
  `registration_descriptor` (binding §2.1) tells receivers the bytes are
  AV1 rather than KLV-async on the same stream_type byte.

```rust,no_run
use tst_core::mpegts::mux::{Config, KlvStreamType, Muxer, StreamSpec, VideoCodec};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cfg = Config {
        streams: vec![
            StreamSpec::Video { pid: 0x1011, codec: VideoCodec::Av1 },
            StreamSpec::Klv {
                pid: 0x1031,
                stream_type: KlvStreamType::PrivateData,
                carries_pts: false,
            },
        ],
        ..Config::default()
    };
    let mut mux = Muxer::new(cfg)?;
    // `au_obus` is a contiguous OBU sequence (each with obu_has_size_field=1).
    // The example builds one synthetic Sequence Header + Temporal Delimiter +
    // Frame access unit; real consumers feed the encoder's output verbatim.
    let au_obus: Vec<u8> = vec![/* concatenated OBUs */];
    mux.push_video(&au_obus, 0, /* key_frame = */ true)?;
    Ok(())
}
```

Runnable: [../crates/tst-srt/examples/mux_av1_with_klv.rs](../crates/tst-srt/examples/mux_av1_with_klv.rs).

### 24. Pair sync-KLV with video AUs via `Pairer::nearest_pts` (Realtime)

The cookbook recipe 12 inline pattern in ~20 lines, expressed through
the opt-in `tst_pipeline::pairing::Pairer`. Same semantics, with
bounded KLV history, telemetry counters, and typed projection structs
on the output.

```rust,no_run
use std::fs;
use tst_core::mpegts::demux::Demuxer;
use tst_pipeline::{MatchMode, Pairer, PairerOutput};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let bytes = fs::read("input.ts")?;
    let mut demux = Demuxer::new();
    demux.feed(&bytes)?;
    demux.flush();

    let mut pairer = Pairer::nearest_pts(
        0x100,        // video PID (discover from ProgramMap)
        0x102,        // KLV PID
        27_000,       // 0.3 s @ 90 kHz tolerance
        32,           // ~1 s history at 30 Hz cadence
        MatchMode::Realtime,
    );
    while let Some(e) = demux.next_event() {
        for o in pairer.feed(e) {
            if let PairerOutput::Paired { video, klv } = o {
                // video.payload → decoder (Annex-B reconstitute, recipe 18)
                // klv.payload   → tst_core::klv::st0601::decode
                let _ = (video, klv);
            }
        }
    }
    let _ = pairer.flush();
    println!("{:?}", pairer.stats());
    Ok(())
}
```

Runnable: `cargo run --example pair_klv_pipeline -- input.ts`.

### 25. Pair sync-KLV in batch mode (`MatchMode::Buffered`)

When KLV PES is interleaved *after* its matching video PES (some
encoders), Realtime mode misses the pairing. Buffered mode holds video
briefly to look ahead.

```rust,no_run
use tst_pipeline::{MatchMode, Pairer};

let mut pairer = Pairer::nearest_pts(
    0x100,
    0x102,
    27_000,
    32,
    MatchMode::Buffered { max_video_buffer: 60 },  // ≈2 s @ 30 fps
);
// feed loop unchanged from recipe 24.
```

Trade-off: up to ~2 s pairing-induced latency in exchange for picking
up otherwise-lost matches. Pick `Realtime` if you can't tolerate the
delay.

### 26. Sample-and-hold async KLV via `Pairer::last_before_pts`

Replaces the cookbook recipe 13 inline pattern. Each video frame
attaches the most recent KLV at `klv.pts <= video.pts`.

```rust,no_run
use tst_pipeline::{Pairer, PairerOutput};
# fn demux_events() -> impl Iterator<Item = tst_core::mpegts::demux::DemuxEvent> { std::iter::empty() }

let mut pairer = Pairer::last_before_pts(
    0x100, // video PID
    0x102, // async-KLV PID
    Some(180_000), // 2 s freshness ceiling — drop pair if KLV is staler
);
for e in demux_events() {
    for o in pairer.feed(e) {
        match o {
            PairerOutput::Paired { video, klv } => { let _ = (video, klv); }
            PairerOutput::UnpairedVideo(_) => { /* KLV too stale or never seen */ }
            _ => {}
        }
    }
}
let _ = pairer.flush();
```

Pass `freshness_ticks = None` to attach regardless of staleness
(matches cookbook recipe 13 default behavior).

### 27. EO + IR composition with shared async-KLV

Two video PIDs sharing one async-KLV PID. Recipe 14's inline
`Option<Vec<u8>>` pattern remains valid; this recipe shows the same
shape via two `Pairer` instances (one per video PID).

```rust,no_run
use tst_pipeline::{Pairer, PairerOutput};
# fn demux_events() -> impl Iterator<Item = tst_core::mpegts::demux::DemuxEvent> { std::iter::empty() }

const EO_PID: u16 = 0x100;
const IR_PID: u16 = 0x101;
const KLV_PID: u16 = 0x102;

let mut eo_pairer = Pairer::last_before_pts(EO_PID, KLV_PID, None);
let mut ir_pairer = Pairer::last_before_pts(IR_PID, KLV_PID, None);

for e in demux_events() {
    // KLV events go to BOTH pairers (each maintains its own slot
    // mark-used state); video events only to the matching pairer.
    let outputs = match &e {
        tst_core::mpegts::demux::DemuxEvent::Metadata { stream, .. }
            if stream.pid == KLV_PID => {
            let mut o = eo_pairer.feed(e.clone());
            o.extend(ir_pairer.feed(e));
            o
        }
        tst_core::mpegts::demux::DemuxEvent::Sample { stream, .. }
            if stream.pid == EO_PID => eo_pairer.feed(e),
        tst_core::mpegts::demux::DemuxEvent::Sample { stream, .. }
            if stream.pid == IR_PID => ir_pairer.feed(e),
        _ => Vec::new(),
    };
    for o in outputs {
        match o {
            PairerOutput::Paired { video, klv } => {
                match video.stream.pid {
                    EO_PID => { /* EO-paired */ let _ = (video, klv); }
                    IR_PID => { /* IR-paired */ let _ = (video, klv); }
                    _ => unreachable!(),
                }
            }
            _ => {}
        }
    }
}
```

Compared to recipe 14's inline pattern, the Pairer-based composition
adds telemetry counters per branch and the typed output projections,
at the cost of one extra clone per KLV event (acceptable for typical
1–10 KB ST 0601 records).

### 28. Decode security metadata from an ST 0601 record

Sibling-layer composition: decode the parent ST 0601 LS, then if
Tag 48 is non-empty, run `klv::st0102::decode` on the inner bytes.

```rust
use tst_core::klv::{st0102, st0601};

# fn process(record_bytes: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
let parent = st0601::decode(record_bytes)?;

if let Some(security_bytes) = parent.security_local_set.as_deref() {
    let security = st0102::decode(security_bytes)?;
    println!(
        "classification={:?} country={:?} version={:?}",
        security.security_classification,
        security.classifying_country,
        security.version,
    );
}
# Ok(())
# }
```

Use `st0102::decode_strict` instead when the consumer wants the spec's
required-tag set enforced (e.g. compliance pipelines for classified
delivery).

Construct + encode the symmetric path:

```rust
use tst_core::klv::st0102::{
    self, ClassifyingCountryCodingMethod, ObjectCountryCodingMethod,
    SecurityClassification, SecurityLs,
};

let security = SecurityLs {
    security_classification: Some(SecurityClassification::Confidential),
    classifying_country_coding_method: Some(
        ClassifyingCountryCodingMethod::Iso3166ThreeLetter,
    ),
    classifying_country: Some("//USA".to_string()),
    object_country_coding_method: Some(
        ObjectCountryCodingMethod::Iso3166ThreeLetter,
    ),
    object_country_codes: Some("USA".to_string()),
    version: Some(12),
    ..Default::default()
};
let bytes = st0102::encode_to_vec(&security)?;
// Stuff `bytes` into a UasDatalinkLs.security_local_set field, then
// st0601::encode_to_vec the parent record.
```

See `examples/decode_security_metadata.rs` for the full file-walking
example.

### 29. Pull sample rate and channel count out of an audio stream

**Goal:** Inspect a `.ts` file and report the typed audio metadata
(sample rate, channel count, codec layer/profile) per audio PID, logging
only the points where the metadata changes.

**Pattern:** Demux to events, dispatch on `SamplePayload::Audio { codec,
frames }`, run `codec::mpegaudio::frames` or `codec::aac::frames` on the
PES blob.

```rust
use tst_core::codec;
use tst_core::mpegts::demux::{AudioCodec, Demuxer, DemuxEvent, SamplePayload};

let bytes = std::fs::read("input.ts")?;
let mut demuxer = Demuxer::new();
demuxer.feed(&bytes)?;
demuxer.flush();

while let Some(ev) = demuxer.next_event() {
    if let DemuxEvent::Sample {
        stream,
        payload: SamplePayload::Audio { codec, frames, .. },
        ..
    } = ev {
        let pid = stream.pid;
        match codec {
            AudioCodec::Mp2 => {
                for f in codec::mpegaudio::frames(&frames).filter_map(Result::ok) {
                    println!("PID 0x{:04x} {:?} {} Hz, {} ch", pid, f.layer, f.sample_rate_hz, f.channels);
                }
            }
            AudioCodec::Aac => {
                for f in codec::aac::frames(&frames).filter_map(Result::ok) {
                    println!("PID 0x{:04x} AAC {:?} {} Hz, {} ch", pid, f.profile, f.sample_rate_hz, f.channels);
                }
            }
            _ => {}
        }
    }
}
```

**Runnable variant:** `cargo run --example parse_audio_frames -- input.ts`
deduplicates output to first-change-only per PID.

**Caveats:**
- `filter_map(Result::ok)` skips parse errors silently. For first-error-stop
  semantics, use `.collect::<Result<Vec<_>, _>>()`.
- Mislabeled-private PIDs in real-world captures may yield mostly
  `BadSyncWord` errors (the source data is private, not audio).
- Silent audio still produces header-valid frames; iterator output ≠ "is
  this audio actually audible."
