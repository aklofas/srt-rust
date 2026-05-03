# Architecture

## Introduction

This document covers how `srt-rust` is laid out: the crate graph, the
internal structure of `srt-core`, and the pipeline composition model that
ties the muxer, transport, and reconnect behaviour together. It targets
evaluators sizing up the project and contributors finding their way around;
integrators may want it as background but should start at
[getting-started.md](getting-started.md) if they just want to use the
library.

The vocabulary established here — `Transport`, `RecvTransport`, "sender
shell", "receive shell", the layering rule — is reused by the per-module
guides ([guide-srt.md](guide-srt.md), [guide-klv.md](guide-klv.md),
[guide-mpegts-mux.md](guide-mpegts-mux.md),
[guide-mpegts-demux.md](guide-mpegts-demux.md),
[guide-pipeline.md](guide-pipeline.md)). Read this first if you plan to
read more than one of those.

## Crate graph

```
srt-sys (raw FFI)  ──→  srt-core  ──→  srt-c (cdylib + staticlib + cbindgen)
                              │
                              ├──→  srt-jni    (planned)
                              └──→  srt-uniffi (planned)

vendored: vendor/srt (libsrt 1.5.5), vendor/mbedtls (3.6.6 LTS)
```

The layering rule is one-directional: lower layers do not depend on upper
layers. Binding crates (`srt-c`, `srt-jni`, `srt-uniffi`) depend on
`srt-core` only — never on `srt-sys` directly. This keeps every binding's
surface area defined by the same safe Rust API and means a fix in
`srt-core` reaches every binding without per-binding patches.

`srt-sys` is the raw FFI layer — bindgen-generated against libsrt 1.5.5,
exposing roughly 72 `srt_*` functions and the full `SRT_*` constant
surface, with `mbedtls` wired in as the encryption backend by default.
`srt-core` is the safe Rust API; nothing above it should ever pull
`srt-sys` into its dependency graph. The vendored libsrt and mbedTLS
submodules are pinned by tag (`v1.5.5`, `v3.6.6` LTS); submodule advances
are deliberate, separate commits. Both vendored builds link statically,
so `srt-c`'s shared library has no runtime dependency on a system libsrt
or libmbedtls.

## Inside `srt-core`: four modules, four jobs

- `srt::*` — safe wrapper for libsrt sockets and listeners.
- `klv::*` — KLV codec, generic substrate plus typed ST 0601 / ST 0605 / ST 1910 layers.
- `mpegts::mux::*` — sender-side MPEG-TS muxer for H.264 / H.265 + KLV.
- `mpegts::demux::*` — receiver-side MPEG-TS demuxer; bytes in, typed `DemuxEvent` out.
- `pipeline::*` — composition of the above into ergonomic sender + receiver shells.

Each module is independently usable. A consumer who only needs KLV decode
can pull in `srt-core` and use `klv::st0601::decode` without touching the
muxer or transport. A consumer who already has their own TS muxer can
skip `mpegts::mux` and feed bytes through `pipeline::TsSender`. A
consumer who only wants the SRT socket — for an entirely different
streaming protocol on top — can use `srt::Socket` and `srt::Listener`
directly.

`pipeline::*` is the only module that depends on the other four. It is
deliberately the thinnest layer in the crate: its job is composition, not
new behaviour. The send shells delegate framing to `mpegts::mux::Muxer`
and wire transport to a `Transport` implementation; the receive shells
delegate framing recovery to `mpegts::demux::Demuxer` and bytes-in to a
`RecvTransport` implementation. Metadata typing for both directions is
`klv::st0601` / `klv::st0605` / `klv::st1910`. The canonical
`Transport` + `RecvTransport` impl is `SrtTransport` over `srt::Socket`
— the same wrapper handles both directions on a connected socket.

## The pipeline composition model

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

Any sender shell composes with any `Transport` implementation. The shells
differ by what they accept on the input side — NAL units plus KLV blobs
(`Sender`), pre-muxed TS bytes (`TsSender`), or arbitrary byte-blind
messages (`RawSender`). The transport differs by what it talks to on the
output side — SRT (`SrtTransport`), a custom UDP socket, a file, an
in-memory buffer for tests. The two axes are orthogonal: you pick a shell
based on what you have, a transport based on where it's going, and they
plug together.

`ManagedTransport<T>` is itself a `Transport` implementation — it
implements the trait by delegating to an inner `T` and adding reconnect
plus a gap buffer that holds messages while the inner transport is down.
Because it satisfies `Transport`, it slots between any sender shell and
the underlying transport without the shell needing to know about
reconnect at all. The shell sees a `Transport`; whether that `Transport`
is plain `SrtTransport` or `ManagedTransport<SrtTransport>` is a
construction-time choice.

For worked examples, see [examples/managed_reconnect.rs](../crates/srt-core/examples/managed_reconnect.rs)
(reconnect + gap buffer in action) and
[examples/custom_transport.rs](../crates/srt-core/examples/custom_transport.rs)
(implementing the `Transport` trait against something other than SRT).

## Why three sender shells

Decision tree:

- You have NAL units plus KLV blobs → `Sender` (auto-muxes through `Muxer`, internally synchronized).
- You have pre-muxed TS bytes → `TsSender` (3-byte sync verify, 7-packet bundling, RECOVER or STRICT framing mode).
- You have arbitrary byte-blind messages → `RawSender` (one `send` call = one outbound SRT message).

There are three shells rather than one because the contracts differ.
`Sender` enforces NAL unit boundaries and KLV record boundaries — it
needs to know where one ends and the next begins to mux them into a TS
stream correctly. `TsSender` enforces TS sync alignment — it needs to
know where each 188-byte packet starts so it can bundle them into SRT
messages of the right shape. `RawSender` enforces nothing beyond the
`SRTO_PAYLOADSIZE` size cap — the caller has already framed the bytes.
Fusing the three into one API would force a least-common-denominator
contract that satisfies none of the use cases well. The shells also
differ in what they do on transient failure. `Sender` carries an
in-flight buffer and replays buffered bytes on reconnect, so a brief
outage is invisible to the receiver above the transport gap-buffer
window. `TsSender` re-establishes sync alignment on the byte stream
after a transport failure — RECOVER mode auto-resyncs to the next sync
byte, STRICT mode fails fast. `RawSender` has no recovery contract by
construction; one `send` either lands as one SRT message or returns an
error. The full mechanics of each shell are covered in
[guide-pipeline.md](guide-pipeline.md).

## The receive pipeline

```
              ┌───────────────────────────────────────┐
              │  RawReceiver  /  TsReceiver           │
              │  Receiver  (full demux)               │
              │  generic over R: RecvTransport        │
              └──────────────┬────────────────────────┘
                             │ R: RecvTransport
              ┌──────────────┴────────────────────────┐
              │  ManagedReceiveTransport<R>           │  (optional)
              │  reconnect on Closed/Broken           │
              └──────────────┬────────────────────────┘
                             │ R: RecvTransport
              ┌──────────────┴────────────────────────┐
              │  SrtTransport (canonical)             │
              │  Custom RecvTransport impl (yours)    │
              └───────────────────────────────────────┘
```

The receive side mirrors the send side. Three shells differ by what
they emit: `RawReceiver::recv_one` returns one byte vec per call (no
TS framing), `TsReceiver::next_packet` emits one 188-byte aligned TS
packet per call (sync recovery internal), `Receiver::recv_event` emits
one typed `DemuxEvent` per call (full TS sync + PSI parse + PES
reassembly + NAL split + KLV unwrap). All three are generic over the
`RecvTransport` trait — the receive-side counterpart to `Transport`,
exposing `recv_bytes`, `max_payload`, `is_alive`, and `close`.
`SrtTransport` implements both `Transport` and `RecvTransport`, so the
same wrapper handles both directions on a connected `srt::Socket`.

`Receiver` is the canonical full-demux shell. It composes
`TsReceiver → Demuxer` internally and exposes a single `recv_event`
draining loop. It also implements `Iterator<Item = Result<DemuxEvent,
ReceiverError>>` so the idiomatic drain pattern is `for result in &mut rx`.
Iterator termination is the clean-EOF signal: when the underlying
`RecvTransport` returns `TransportError::Closed`, `Receiver` calls
`Demuxer::flush` (recovering any trailing PES) and returns `Ok(None)`,
which the iterator turns into `None`. Peer-disconnect or unrecoverable
link errors surface as `Err(ReceiverError::Transport(Broken(_)))`;
strict-mode rejections surface as `Err(ReceiverError::Demux(_))`.

`Receiver::add_byte_sink` is the fan-out hook: register a callback
(`Box<dyn FnMut(&[u8]) + Send>`), and the callback receives every
188-byte TS packet pulled from the transport — in registration order,
before the demuxer parses them. The canonical use case is a
"write-to-disk + forward-via-RTP + demux-for-KLV" workflow where
multiple consumers tee off the same byte stream in one pass.

`ManagedReceiveTransport<R>` is the receive-side reconnect decorator,
sibling to `ManagedTransport<T>`. It implements `RecvTransport`, so it
slots between any receive shell and the underlying transport
transparently. **It has no gap buffer** — receive-side bytes that
never arrived can't be replayed, so the decorator simply restarts the
recv loop on a fresh transport when the inner returns `Closed` or
`Broken`. The demuxer-side state (sync alignment, PES reassembly) does
carry over across reconnect, which costs at most one re-VERIFY pass
on the syncer.

**Decoupled pairing.** The demuxer surfaces every video AU and every
KLV record as an independent stream-tagged event with full timing.
It does **not** pair sync-KLV with video AUs — pairing tolerance,
sample-and-hold semantics, and multi-stream routing are
consumer-domain decisions the library can't make correctly for
everyone. The three canonical pairing patterns live as cookbook
recipes (12, 13, 14) with runnable example companions
(`pair_sync_klv.rs`, `tee_disk_and_demux.rs`).

## Sync vs. async

The public API is sync blocking. `Socket::send` and `Socket::recv` block
the calling thread; `Sender::send_video` blocks until the underlying SRT
socket has accepted the bytes; reconnect inside `ManagedTransport` runs
on the caller's thread.

Sync was chosen for three reasons. First, the target deployment shape
is small — a process talks to ≤10 SRT peers, so the thread-per-
connection cost is negligible. Second, sync code is simpler to reason
about and debug, especially when bridging into C / JVM / Swift / Kotlin
through the binding crates, none of which have a portable async story.
Third, the sync API mirrors `std::net::TcpStream` semantics, so a Rust
caller already knows the shape from the standard library.

Async is on the deferred-features list, not ruled out. Two viable paths
exist when a consumer asks. The lightweight path is a Tokio integration
that wraps each blocking call in `spawn_blocking` — minimal extra surface,
acceptable for low connection counts, no changes to `srt-core`'s
internals. The heavier path is a full async reactor backed by libsrt's
`srt_epoll_*` family with `tokio::io::unix::AsyncFd` or equivalent
registration — better scalability, much bigger surface to design and
test. The choice is consumer-driven; until then the sync API stays. See
[`docs/deferred-features.md`](deferred-features.md) for the current note.

Note that "sync" applies to the public API surface — internally, the
`Sender` family uses an internal mutex so the data path is safe to call
from multiple threads if a consumer wants to fan in NAL and KLV inputs
on separate producers. The synchronization is invisible to single-
threaded callers and adds no contention when only one thread is
producing.

## What's deferred

The summary below points at the canonical list once; each item maps to
an entry in [`docs/deferred-features.md`](deferred-features.md).

- Audio carriage in `mpegts::mux` and typed audio in `mpegts::demux` — video + KLV only today; `SamplePayload::Audio` reserved for additive lift.
- Subtitle, caption, and auxiliary-data channels — same shape as audio; `SamplePayload::Subtitle` reserved.
- AV1 / H.266 codec variants — surface as `SamplePayload::Unknown` today; OBU-shaped (AV1) variant requires a cross-codec rework.
- Multi-program TS in `mpegts::demux` — single PMT only today; `ProgramMap.program_number` carries the number for additive lift.
- `pipeline::pairing` — opt-in convenience pairing utility; cookbook recipes 12–14 are the canonical patterns until consumers ask for shared substrate.
- Reactor / async / `srt_epoll_*` — see the sync-vs-async section above.
- Bonding / connection groups (`SRTO_GROUP*`) — no consumer demand.
- Other typed MISB sets — ST 0903 VMTI, ST 0806, ST 0102 typed view; pass-through today.
- `serde` / `no_std` for `klv` — pure additive; behind feature flags when added.
- Rustdoc lift to docs.rs — these markdown files are written CommonMark-clean so the lift is mechanical when scheduled.

See [`docs/deferred-features.md`](deferred-features.md) for the
canonical list and the rationale for each entry.
