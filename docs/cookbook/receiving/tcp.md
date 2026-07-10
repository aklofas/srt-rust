# Receive MPEG-TS over TCP

```rust
use tst_pipeline::DemuxReceiver;
use tst_tcp::TcpListener;

let listener = TcpListener::bind("0.0.0.0:7001".parse()?)?;
let rx = listener.accept_blocking()?;  // wait for caller
let mut receiver = DemuxReceiver::new(rx);

for event in &mut receiver {
    match event? {
        // ... handle DemuxEvent variants
        _ => {}
    }
}
```

## Listener URL form

You can also bind via URL with `?listen=1`:

```rust
use tst_tcp::TcpListener;

let listener = TcpListener::from_url("tcp://0.0.0.0:7001?listen=1&nodelay=1")?;
let rx = listener.accept_blocking()?;
```

For TLS listeners, the bind address must be an IP literal because the OS must
bind to a specific address (use `0.0.0.0` or `::` to listen on all interfaces).
The server certificate must match what callers dial: a `dnsName` SAN if callers
use a hostname, or an `iPAddress` SAN if callers use an IP literal. See the
[TLS section in the TCP send guide](/docs/cookbook/sending/tcp.md) for OpenSSL
one-liners to generate certs for both cases.

```rust
let listener = TcpListener::from_url(
    "tcps://0.0.0.0:7001?listen=1&cert=server.crt&key=server.key"
)?;
```

## URL parameters (listener-side)

| Parameter | Default | Meaning |
|---|---|---|
| `listen` | (required for listener) | Set to `1` to indicate listener intent |
| `nodelay` | `1` (enabled) | TCP_NODELAY for accepted connections; Nagle is disabled by default (`0` re-enables it) |
| `rcvbuf` | OS default | SO_RCVBUF for accepted connections |
| `sndbuf` | OS default | SO_SNDBUF for accepted connections |
| `keepalive` | disabled | SO_KEEPALIVE idle time in seconds |
| `cert` | (required for `tcps://`) | Server certificate PEM path (TLS listener) |
| `key` | (required for `tcps://`) | Server private key PEM path (TLS listener) |

## Caller-side receive

Symmetric — `TcpTransport::connect` returns a handle that also impls
`RecvTransport`, useful when the receiver is the caller (e.g., behind a NAT
that allows outbound TCP):

```rust
use tst_pipeline::DemuxReceiver;
use tst_tcp::TcpTransport;

let rx = TcpTransport::connect("tcp://192.168.1.10:7001")?;
let mut receiver = DemuxReceiver::new(rx);
```

## See also

- [Send MPEG-TS over TCP](../sending/tcp.md)
- [Receive MPEG-TS over UDP](udp.md)
