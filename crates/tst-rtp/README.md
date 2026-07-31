# tst-rtp

RTP-over-UDP carrying MPEG-TS (RFC 2250) or a bare H.264 elementary
stream (RFC 6184), with an RTSP/1.0 + RTSP/2.0 client and server for
session negotiation. Pure Rust, no native dependencies.

Built on `tst-core`'s transport traits; pair with `tst-pipeline`'s
`MuxSender`/`DemuxReceiver` shells for the MPEG-TS-over-RTP path. See
the [docs landing
page](https://github.com/aklofas/ts-transformer/blob/main/docs/index.md)
for the full guide set.

## Quick start

```rust,no_run
use tst_core::transport::Transport;
use tst_rtp::RtpTransport;

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let mut tx = RtpTransport::connect("rtp://239.55.55.1:5004?ttl=8")?;

let ts_packet = [0u8; 188]; // caller-supplied MPEG-TS packet
tx.send_bytes(&ts_packet)?;
# Ok(())
# }
```

**Stability: Stable** for the RTP transports and `RtspClient`; the
`rtsp` module's server surface (`RtspServer`) is **Provisional** —
still evolving. See the [API stability
reference](https://github.com/aklofas/ts-transformer/blob/main/docs/reference/api-stability.md).

**License:** MIT OR Apache-2.0.
