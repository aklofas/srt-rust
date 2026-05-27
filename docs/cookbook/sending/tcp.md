# Send MPEG-TS over TCP

Raw MPEG-TS over TCP is the reliable-bytestream sibling of UDP. Useful when
packet loss matters more than latency, or when firewall topology blocks
multicast. `ffmpeg -listen 1 -i tcp://...` and GStreamer's `tcpsink` are
common counterparts on the receiver side.

## Code

```rust
use tst_core::mpegts::mux::MuxerConfig;
use tst_pipeline::MuxSender;
use tst_tcp::TcpTransport;

let tx = TcpTransport::connect("tcp://127.0.0.1:7001?nodelay=1")?;
let cfg = MuxerConfig::builder()
    // ... configure programs/streams as needed
    .build()?;
let mut sender = MuxSender::new(tx, cfg)?;
sender.send_video(&nal_bytes, pts, key_frame)?;
sender.send_klv(&klv_bytes, pts)?;
```

## TLS (`tcps://`)

```rust
use tst_tcp::TcpTransport;

// Uses OS native cert store by default (no webpki-roots dep).
let tx = TcpTransport::connect("tcps://video.example.com:7001")?;

// Or supply a custom CA bundle:
let tx = TcpTransport::connect("tcps://video.example.com:7001?ca=ca-bundle.pem")?;
```

## URL parameters

| Parameter | Default | Meaning |
|---|---|---|
| `nodelay` | OS default | TCP_NODELAY (`1` = disable Nagle for low-latency streaming) |
| `keepalive` | disabled | SO_KEEPALIVE idle time in seconds |
| `rcvbuf` | OS default | SO_RCVBUF in bytes (suffixes: `K`, `M`) |
| `sndbuf` | OS default | SO_SNDBUF in bytes |
| `connect_timeout` | 10 | Caller-side connect timeout in seconds |
| `ca` | OS native store | Custom CA bundle PEM path (TLS caller only) |

## Verify with ffmpeg

```bash
# Listener first:
ffmpeg -listen 1 -i tcp://0.0.0.0:7001 -c copy out.ts

# Then caller (from our send_tcp example):
cargo run -p tst-examples --example send_tcp -- input.ts tcp://127.0.0.1:7001
```

## See also

- [Receive MPEG-TS over TCP](../receiving/tcp.md)
- [Send MPEG-TS over UDP](udp.md) — when latency matters more than reliability
- [Send MPEG-TS over SRT](11-sender-from-url.md) — for reliable + encrypted transport with FEC
