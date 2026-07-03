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

`tcps://` requires an **IP literal** — the URL parser accepts only IPv4/IPv6
addresses, not hostnames. The TLS library presents the IP address to the server
during the handshake, so the server certificate must carry a matching
`iPAddress` SubjectAltName (SAN). A certificate with only a `dnsName` SAN (even
if the DNS name resolves to that IP) will be rejected with a TLS certificate
error.

Generate a certificate with an IP SAN using OpenSSL:
```bash
openssl req -x509 -nodes -newkey rsa:2048 \
  -subj "/CN=server" \
  -addext "subjectAltName=IP:192.168.1.10" \
  -out server.crt -keyout server.key
```

```rust
use tst_tcp::TcpTransport;

// Uses OS native cert store (the CA that signed the server cert must be there).
let tx = TcpTransport::connect("tcps://192.168.1.10:7001")?;

// Or supply a custom CA bundle (e.g. a self-signed CA):
let tx = TcpTransport::connect("tcps://192.168.1.10:7001?ca=ca-bundle.pem")?;
```

## URL parameters

| Parameter | Default | Meaning |
|---|---|---|
| `nodelay` | `1` (enabled) | TCP_NODELAY; disable Nagle by default for low-latency streaming (`0` re-enables Nagle for bulk throughput) |
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
