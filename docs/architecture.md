# Architecture

## Introduction

This document covers how `srt-rust` is laid out: the crate graph, the
internal structure of `srt-core`, and the pipeline composition model that
ties the muxer, transport, and reconnect behaviour together. It targets
evaluators sizing up the project and contributors finding their way around;
integrators may want it as background but should start at
[getting-started.md](getting-started.md) if they just want to use the
library.

The vocabulary established here — `Transport`, "sender shell", "v0 sender
pipeline", the layering rule — is reused by the per-module guides
([guide-srt.md](guide-srt.md), [guide-klv.md](guide-klv.md),
[guide-mpegts-mux.md](guide-mpegts-mux.md),
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
- `pipeline::*` — composition of the above into ergonomic sender shells.

Each module is independently usable. A consumer who only needs KLV decode
can pull in `srt-core` and use `klv::st0601::decode` without touching the
muxer or transport. A consumer who already has their own TS muxer can
skip `mpegts::mux` and feed bytes through `pipeline::TsSender`. A
consumer who only wants the SRT socket — for an entirely different
streaming protocol on top — can use `srt::Socket` and `srt::Listener`
directly.

`pipeline::*` is the only module that depends on the other three. It is
deliberately the thinnest layer in the crate: its job is composition, not
new behaviour. The shells delegate framing to `mpegts::mux::Muxer`,
metadata typing to `klv::st0601` / `klv::st0605` / `klv::st1910`, and
wire transport to a `Transport` implementation — usually
`SrtTransport` over `srt::Socket`.

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

## Sync vs. async

The v0 API is sync blocking. `Socket::send` and `Socket::recv` block the
calling thread; `Sender::send_video` blocks until the underlying SRT
socket has accepted the bytes; reconnect inside `ManagedTransport` runs
on the caller's thread.

Sync was chosen for v0 for three reasons. First, the target deployment
shape is small — a process talks to ≤10 SRT peers, so the thread-per-
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
the deferred-features entry "Reactor / `srt_epoll_*` exposure" in the
parent workspace doc at `~/Projects/srt/docs/deferred-features.md` (not
part of the published repo) for the current note.

Note that "sync" applies to the public API surface — internally, the
`Sender` family uses an internal mutex so the data path is safe to call
from multiple threads if a consumer wants to fan in NAL and KLV inputs
on separate producers. The synchronization is invisible to single-
threaded callers and adds no contention when only one thread is
producing.

## What's deferred

The deferred-features doc lives in the parent workspace at
`~/Projects/srt/docs/deferred-features.md`. It is intentionally outside
the published repo because it tracks design-state context the public
artifact does not need to carry. The summary below points at it once;
each item below maps to an entry there.

- `mpegts::demux` — receiver-side TS demuxer. Receivers use FFmpeg / JavaCV / Bento4 / platform demuxers.
- Audio carriage in `mpegts::mux` — video + KLV only today.
- Subtitles, captions, and other PMT entries — out of scope; PMT shape is video + KLV.
- `pipeline::receiver` — receive-side pipeline shell; depends on `mpegts::demux` shipping first.
- Reactor / async / `srt_epoll_*` — see the sync-vs-async section above.
- Bonding / connection groups (`SRTO_GROUP*`) — no consumer demand.
- Other typed MISB sets — ST 0903 VMTI, ST 0806, ST 0102 typed view; pass-through today.
- `serde` / `no_std` for `klv` — pure additive; behind feature flags when added.
- Rustdoc lift to docs.rs — these markdown files are written CommonMark-clean so the lift is mechanical when scheduled.

See `~/Projects/srt/docs/deferred-features.md` for the canonical list and
the rationale for each entry.

## Where the design specs live

Architecture decisions, design rationale, and per-plan implementation
notes live at `~/Projects/srt/docs/specs/` and `~/Projects/srt/docs/plans/`
in the parent workspace — not in this published repo. The split is
deliberate: the parent workspace tracks the project's full design state
(research, prior-art analyses, decision logs, plan-by-plan execution
records); the `srt-rust` repo carries only what a consumer or contributor
needs to use or extend the shipping artifact. Contributors who want to
read the architecture rationale, scope-discipline trail, or pre-ship plan
documents should look in the parent workspace.
