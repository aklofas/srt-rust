# Receive MPEG-TS over UDP

```rust
use tst_pipeline::DemuxReceiver;
use tst_udp::UdpRecvTransport;

let rx = UdpRecvTransport::listen("udp://@239.10.0.1:5004?iface=eth0&rcvbuf=8M")?;
let mut receiver = DemuxReceiver::new(rx);

for event in &mut receiver {
    match event? {
        // ... handle DemuxEvent variants
        _ => {}
    }
}
```

## URL parameters

| Parameter | Default | Meaning |
|---|---|---|
| `iface` | OS default | Multicast join interface (IP addr or interface name) |
| `rcvbuf` | OS default | SO_RCVBUF in bytes (`8M` typical for high-bitrate streams) |
| `pkt_size` | 1316 | Recv buffer hint per call |

For unicast bind, prefix the host with `@` to make intent explicit:
`udp://@0.0.0.0:5004`.

## See also

- [Send MPEG-TS over UDP](../sending/udp.md)
