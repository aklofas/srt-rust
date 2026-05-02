# Pipeline Composition Guide

## Introduction

This guide covers `srt_core::pipeline` — the composition layer that
wires `mpegts::mux::Muxer`, the KLV codecs, and `srt::Socket` into
ergonomic sender shells. The shells are thin: their job is to glue
framing, metadata typing, and wire transport together with a stable
contract, not to invent new behaviour.

Reach for `pipeline::*` when you want a ready-made path from NAL units
plus KLV blobs (or pre-muxed TS bytes, or arbitrary application
messages) to an SRT socket, optionally with reconnect and a gap
buffer. Reach past it — straight to `mpegts::mux::Muxer` or
`srt::Socket` — when you have a specialised need (custom buffering,
non-SRT wire, your own reconnect strategy).

For the higher-level composition story, see
[architecture.md](architecture.md). This guide assumes that vocabulary.

## The composition model

```
                   ┌────────────────────────────────┐
                   │  Sender / TsSender / RawSender │  (3 sender shells)
                   │  generic over T: Transport     │
                   └──────────────┬─────────────────┘
                                  │ T: Transport
              ┌───────────────────┴───────────────────┐
              │  ManagedTransport<T>  (decorator)     │  (optional)
              │  reconnect + gap buffer               │
              └───────────────────┬───────────────────┘
                                  │ T: Transport
              ┌───────────────────┴───────────────────┐
              │  SrtTransport (canonical)             │
              │  Custom Transport impl (yours)        │
              └───────────────────────────────────────┘
```

The two axes are orthogonal. Shells differ by what they accept on the
input side; transports differ by what they talk to on the output side.
Pick a shell based on what you have, a transport based on where it's
going, and they plug together.

## Picking a sender

Decision tree:

- **NAL units plus KLV blobs → `Sender`.** Auto-muxes through an
  internal `mpegts::mux::Muxer`; internally synchronized so
  `send_video` and `send_klv` are safe to call concurrently from
  different threads. Lossless across transient transport failures —
  drained-but-not-yet-sent bytes are retained in `pending_bytes` and
  drained first on the next call.
- **Pre-muxed TS bytes → `TsSender`.** 3-byte TS sync verification,
  7-packet bundling for the canonical 1316-byte SRT payload size.
  RECOVER mode auto-resyncs to the next sync byte after loss; STRICT
  mode fails fast on any non-aligned input.
- **Arbitrary byte-blind messages → `RawSender`.** One `send` call
  equals one outbound SRT message of the exact length you passed. No
  buffering, no framing, no accumulation.

See [architecture.md](architecture.md)'s "Why three sender shells" for
the rationale against fusing them.

## `Sender` walkthrough

```rust,ignore
impl<T: Transport> Sender<T> {
    pub fn new(config: Config, transport: T) -> Result<Self, MuxError>;
    pub fn send_video(&self, nal: &[u8], pts_90khz: i64, key_frame: bool)
        -> Result<(), SenderError>;
    pub fn send_klv(&self, klv: &[u8], pts_90khz: i64)
        -> Result<(), SenderError>;
    pub fn close(&self);
    pub fn is_alive(&self) -> bool;
}
```

`SenderError` is two-variant: `Mux(MuxError)` for muxer-side failures
(`BufferFull`, `KlvTooLarge`, `InvalidNal`) and
`Transport(TransportError)` for transport-side failures. Both convert
in via `#[from]`.

An internal `Mutex` wraps the muxer, the transport, and `pending_bytes`.
Concurrent `send_video` / `send_klv` calls are correct but serialize.
The lock is held across push → mux drain → transport send so
back-pressure is honoured end-to-end.

`pending_bytes` is unbounded — the bare `Sender` has no cap on how
many drained-but-unsent chunks accumulate during prolonged transport
unavailability. Wrap with `ManagedTransport` when you expect outages
longer than a fraction of a second.

**ST 1910 AU cell wrapping is caller-side, not sender-side.**
`Sender::send_klv` mirrors `Muxer::push_klv` — it treats the KLV blob
as opaque bytes regardless of how the muxer is configured. When the
underlying `Config` is set for `KlvStreamType::SynchronousMetadata`
plus `carries_pts: true`, wrap with `klv::st1910::wrap_au_cell` before
calling `send_klv`. See [guide-mpegts-mux.md](guide-mpegts-mux.md)
§"KLV-in-TS modes" and [guide-klv.md](guide-klv.md)'s "ST 1910 AU
cell wrap/unwrap" section.

Mirroring [../crates/srt-core/examples/pipeline_send_to_socket.rs](../crates/srt-core/examples/pipeline_send_to_socket.rs):

```rust,no_run
use srt_core::mpegts::mux::Config;
use srt_core::pipeline::{Sender, SrtTransport};
use srt_core::srt::SocketBuilder;
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let socket = SocketBuilder::new()
        .latency(Duration::from_millis(120))
        .connect("127.0.0.1:9000")?;
    let sender = Sender::new(Config::default(), SrtTransport::new(socket))?;
    for i in 0..5i64 {
        let nal = vec![0x00, 0x00, 0x00, 0x01, 0x65, 0xAA];
        let klv = vec![/* ... pre-built ST 0601 ... */];
        sender.send_video(&nal, i * 3000, i == 0)?;
        sender.send_klv(&klv, i * 3000)?;
    }
    sender.close();
    Ok(())
}
```

## `TsSender` walkthrough

```rust,ignore
impl<T: Transport> TsSender<T> {
    pub fn new(transport: T, config: TsSenderConfig) -> Self;
    pub fn send_ts(&mut self, bytes: &[u8]) -> Result<(), TsSenderError>;
    pub fn flush(&mut self) -> Result<(), TsSenderError>;
    pub fn stats(&self) -> &TsSenderStats;
    pub fn close(&mut self);
    pub fn is_alive(&self) -> bool;
}
```

`send_ts` accepts any number of bytes; the sender does 188-alignment
and 7-packet bundling internally. `flush` emits any buffered partial
bundle so the tail of a finite input reaches the wire. Drop also
best-effort flushes.

`TsSenderConfig` has two knobs:

- `framing_mode: TsFramingMode::Recover` (default) silently skips
  misaligned bytes until it finds a TS sync byte (counts them in
  `bytes_skipped_for_sync`); auto-resyncs after sync loss.
- `framing_mode: TsFramingMode::Strict` returns
  `TsFramingError::SyncLost { offset }` on any non-aligned input.
- `max_unsynced_bytes: usize` — bytes consumed while UNSYNCED before
  terminal `TsFramingError::NoSyncAfterLimit`. Default 18,800.

`TsSenderError` is two-variant: `Framing(TsFramingError)` and
`Transport(TransportError)`.

`TsSenderStats` fields: `bytes_pushed`, `bytes_skipped_for_sync`
(bytes discarded while acquiring or re-acquiring sync, RECOVER mode
only), `resync_events`, `packets_sent` — all `u64`.

Mirroring [../crates/srt-core/examples/ts_relay_from_file.rs](../crates/srt-core/examples/ts_relay_from_file.rs):

```rust,no_run
use srt_core::pipeline::{SrtTransport, TsSender, TsSenderConfig};
use srt_core::srt::SocketBuilder;
use std::fs::File;
use std::io::Read;
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let socket = SocketBuilder::new()
        .latency(Duration::from_millis(120))
        .connect("127.0.0.1:9000")?;
    let mut sender = TsSender::new(SrtTransport::new(socket), TsSenderConfig::default());
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

## `RawSender` walkthrough

```rust,ignore
impl<T: Transport> RawSender<T> {
    pub fn new(transport: T, config: RawSenderConfig) -> Self;
    pub fn send(&mut self, bytes: &[u8]) -> Result<(), TransportError>;
    pub fn close(&mut self);
    pub fn is_alive(&self) -> bool;
    pub fn transport(&self) -> &T;
}
```

`RawSenderConfig` is currently empty — reserved as a distinct type so
future additions are non-breaking. `send` validates
`bytes.len() <= transport.max_payload()` before delegating; one `send`
call equals one outbound message and the transport's `TransportError`
surfaces directly.

Use case: custom protocols where you want SRT's reliability and
encryption but not its TS muxing — control channels, application-level
file transfer, single-message latency measurement.

## The `Transport` trait

```rust,ignore
pub trait Transport: Send {
    fn send_bytes(&mut self, msg: &[u8]) -> Result<(), TransportError>;
    fn max_payload(&self) -> usize;
    fn is_alive(&self) -> bool;
    fn close(&mut self);
}
```

The trait is `Send` but not `Sync` — concurrent sends through one
transport are the caller's responsibility. The shells handle this
internally where their thread-safety contract requires it.

`TransportError` has four variants:

- `Backpressure(String)` — transport is alive but momentarily refused
  the bytes. Retrying the same slice is reasonable.
- `Broken(String)` — transport is dead; rebuild it (or rely on
  `ManagedTransport` to do so).
- `Closed` — transport was already closed.
- `TooLarge { len, max }` — message exceeds `max_payload`. Caller is
  responsible for chunking on their own framing semantics.

Implement `Transport` for any byte sink that isn't an SRT socket: UDP,
file, in-memory test harness, named pipe, TCP, your own protocol. The
shells don't care which.

Mirroring [../crates/srt-core/examples/custom_transport.rs](../crates/srt-core/examples/custom_transport.rs):

```rust,no_run
use srt_core::pipeline::{Transport, TransportError};
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

## `SrtTransport` — the canonical impl

```rust,ignore
impl SrtTransport {
    pub const DEFAULT_PAYLOAD: usize = 1316;
    pub fn new(socket: Socket) -> Self;
    pub fn with_max_payload(self, n: usize) -> Self;
}
```

`SrtTransport::new` wraps an already-connected `Socket`. Configure the
socket (passphrase, latency, stream id, etc.) before wrapping —
`SrtTransport` doesn't expose libsrt knobs of its own. `max_payload`
defaults to 1316 (libsrt's `SRTO_PAYLOADSIZE` default) and is NOT
queried from the socket; for a non-default payload size, call
`with_max_payload`.

Error mapping from `SendError` → `TransportError`:

| `SendError` | `TransportError` |
| --- | --- |
| `TimedOut` / `QueueFull` | `Backpressure(...)` |
| `PayloadTooLarge { actual, .. }` | `TooLarge { len: actual, max }` |
| `ConnectionBroken` / `System(_)` | `Broken(...)` (drops socket) |
| `Other { kind: SrtErrno::Async, .. }` | `Backpressure(message)` |
| `Other { .. }` | `Broken(message)` (drops socket) |

When a transport is marked broken, the wrapped `Socket` is dropped
internally — subsequent `send_bytes` calls return `Closed` until a
new `SrtTransport` is built.

## `ManagedTransport<T>` — reconnect + gap buffer

```rust,ignore
impl<T: Transport + 'static> ManagedTransport<T> {
    pub fn new<F>(inner: T, factory: F, policy: ReconnectPolicy) -> Self
    where F: Fn() -> Result<T, TransportError> + Send + Sync + 'static;
}
```

`ManagedTransport<T>` itself implements `Transport`, so any sender
shell composes with it transparently — the shell sees a `Transport`,
plain or wrapped.

The factory closure is what the manager calls to rebuild the inner
transport. Most callers wire it as a closure running
`SocketBuilder::new()...connect(addr).map(SrtTransport::new)`. The
closure must be `Fn + Send + Sync + 'static` so the manager can hold
it in an `Arc<dyn Fn …>`.

Behaviour on `send_bytes`: drain any queued gap-buffer bytes first,
then try the new bytes. `Backpressure` and `TooLarge` propagate to
the caller without triggering reconnect. `Broken` or `Closed` queues
the new bytes into the gap buffer (subject to `OverflowPolicy`) and
calls the factory with the configured backoff until a fresh transport
materialises or `max_attempts` is exhausted. On reconnect the gap
buffer drains before the call returns.

Reconnect runs synchronously on the calling thread — a single
`send_bytes` call may block for the full reconnect window. Async
reconnect is on the deferred-features list; see
[deferred-features.md](deferred-features.md).

## `ReconnectPolicy`

```rust,ignore
pub struct ReconnectPolicy {
    pub max_attempts: Option<u32>,
    pub backoff: BackoffStrategy,
    pub gap_buffer_capacity: usize,
    pub overflow_policy: OverflowPolicy,
}
```

Defaults (`ReconnectPolicy::default()`):

- `max_attempts: Some(10)` — give up after ten reconnect attempts and
  surface `Broken("reconnect gave up after 10 attempts")` to the
  caller. Set to `None` to retry forever.
- `backoff: BackoffStrategy::Exponential { base: 100ms, max: 10s }` —
  see §11 for the actual variants.
- `gap_buffer_capacity: 256` — messages.
- `overflow_policy: OverflowPolicy::DropOldest` — see §12.

## `BackoffStrategy`

Two variants:

- `Constant(Duration)` — fixed wait. Use for tests or controlled
  environments.
- `Exponential { base: Duration, max: Duration }` —
  `wait = base * 2^(attempt - 1)`, capped at `max`. Default
  `base = 100ms`, `max = 10s`. Use for production.

There is no jitter variant. Exponential without jitter can create
thundering-herd issues with many simultaneous clients reconnecting to
the same peer; for the typical single-sender deployment this is fine.

## `OverflowPolicy`

Two variants:

- `DropOldest` (default) — when the gap buffer is full, drop the
  oldest queued message to make room. Counts the drop in
  `messages_dropped` / `bytes_dropped`.
- `Reject` — return `GapBufferError::Full` from the enqueue path; the
  caller decides what to do.

Trade-off: `DropOldest` keeps the receiver caught up to "now" once
reconnect lands, at the cost of losing the tail of the gap — the
right call for live video. `Reject` preserves every message at the
cost of stalling once the disconnect window exceeds the buffer; reach
for it when correctness matters more than freshness (telemetry,
control-plane messages, audit-logged events).

## Choosing a backoff and gap-buffer size

Reasonable starting point: `BackoffStrategy::Exponential { base: 100ms,
max: 10s }` plus

```text
gap_buffer_capacity = ceil(max_disconnect_window × send_rate)
```

Worked example: a five-second worst-case disconnect on a 30 fps stream
with one KLV record per frame produces 150 video messages plus 150 KLV
messages = 300 entries. Round up to 512. The 256-message default fits
roughly 4 seconds of that load.

If your worst-case window is longer than the buffer can hold, either
raise the capacity (cheap — each entry is a `Vec<u8>` of at most
`max_payload` bytes) or accept `DropOldest` as a deliberate
freshness-over-completeness choice.

## Failure semantics

Where errors come from:

- `SenderError::Mux` — the encoder produced something the muxer
  rejects (`BufferFull`, `KlvTooLarge`, `InvalidNal`). Rare in
  practice with reasonable buffer sizing.
- `SenderError::Transport(TransportError)` — the transport layer
  reported an error.
- `TsSenderError::Framing(TsFramingError)` — STRICT mode rejected
  unaligned input, or RECOVER mode burned through `max_unsynced_bytes`.
- `TsSenderError::Transport(TransportError)` — same as above.
- `RawSender::send` returns `TransportError` directly.

With `ManagedTransport` wrapping the inner transport, transient
`Broken` errors are absorbed and the caller's `send_*` call appears to
succeed once reconnect lands. Only `Closed` (after `max_attempts`
exhausted) and `TooLarge` propagate. With a bare transport, every
`Broken` propagates — you reconnect by rebuilding the transport and
re-creating the sender.

## Threading

- `Sender` is internally synchronized. `send_video` and `send_klv` are
  safe to call concurrently; calls serialize on the internal mutex.
- `TsSender` and `RawSender` are not internally synchronized. Wrap in
  an external `Mutex` if shared across threads, or — simpler — use
  thread-per-connection.
- `ManagedTransport` itself is internally synchronized.

Don't over-engineer this. The typical deployment has one sender per
process, or a small fixed number of senders each on its own thread.

## Examples

Four runnable examples cover the pipeline surface:

- [../crates/srt-core/examples/pipeline_send_to_socket.rs](../crates/srt-core/examples/pipeline_send_to_socket.rs)
  — `Sender` → `SrtTransport` → connected SRT socket. The canonical
  setup.
- [../crates/srt-core/examples/ts_relay_from_file.rs](../crates/srt-core/examples/ts_relay_from_file.rs)
  — `TsSender` reading a `.ts` file and relaying to an SRT peer.
- [../crates/srt-core/examples/managed_reconnect.rs](../crates/srt-core/examples/managed_reconnect.rs)
  — `ManagedTransport<SrtTransport>` plus a deliberately flaky peer
  thread; demonstrates the reconnect + gap-buffer + drain cycle
  end-to-end.
- [../crates/srt-core/examples/custom_transport.rs](../crates/srt-core/examples/custom_transport.rs)
  — implementing the `Transport` trait against an in-memory byte
  collector. Template for any non-SRT sink.
