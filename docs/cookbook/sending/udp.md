# Send MPEG-TS over UDP

Raw MPEG-TS over UDP is the lowest-common-denominator broadcast transport.
ffmpeg, VLC, GStreamer, and most STANAG 4609 ground stations consume it
without configuration.

## Code

```rust
use tst_core::mpegts::mux::MuxerConfig;
use tst_pipeline::MuxSender;
use tst_udp::UdpTransport;

let tx = UdpTransport::connect("udp://239.10.0.1:5004?iface=eth0&ttl=8")?;
let cfg = MuxerConfig::builder()
    // ... configure programs/streams as needed
    .build()?;
let mut sender = MuxSender::new(tx, cfg)?;
sender.send_video(&nal_bytes, pts, key_frame)?;
sender.send_klv(&klv_bytes, pts)?;
```

## Verify with ffmpeg

```bash
ffmpeg -i 'udp://@239.10.0.1:5004?fifo_size=1000000' -c copy out.ts
```

For unicast (skip the `@`):

```bash
ffmpeg -i 'udp://0.0.0.0:5004?fifo_size=1000000' -c copy out.ts
```

## URL parameters

| Parameter | Default | Meaning |
|---|---|---|
| `iface` | OS default | Outgoing interface for multicast (IP addr or interface name) |
| `ttl` | OS default (1 for multicast) | IPv4 TTL or IPv6 hop limit |
| `tos` | none | IP TOS / DSCP byte (e.g., `0xb8` for EF) |
| `sndbuf` | OS default | SO_SNDBUF in bytes (suffixes: `K`, `M`) |
| `pkt_size` | 1316 (7×188) | Datagram payload size |

## See also

- [Receive MPEG-TS over UDP](../receiving/udp.md)
- [Send MPEG-TS over SRT](../sending/11-sender-from-url.md) — for reliable/encrypted transport
- [Send MPEG-TS over RTP](../sending/08-custom-transport.md) — for RTP-wrapped UDP
