# SrtCancelHandle — SRT cross-thread shutdown primitive

Every long-lived pipeline shell (`MuxSender`, `Sender`, `RawSender`,
`DemuxReceiver`, `Receiver`, `RawReceiver`) blocks the calling thread
inside `send_*` / `recv_*`. The cancel-handle pattern is how a
*different* thread (or a signal handler) wakes that blocked call so the
process can shut down promptly without time-sliced polling.

The handle is `Send + Sync`, one-shot, and idempotent — multiple
`cancel()` calls from any thread are safe and no-op after the first.

## Two layers, same primitive

There are two named types in play. The trait `TransportCancel` is
transport-agnostic (every `Transport` impl decides whether to surface a
cancel handle); the concrete struct `SrtCancelHandle` is SRT-shaped
(wraps an `SRTSOCKET` integer handle with `i64::MIN` as the cancelled
sentinel). Pick the one whose layer you're on:

| Layer | Type | Where it lives |
|-------|------|----------------|
| Pipeline (trait, dynamic dispatch) | [`TransportCancel`](../crates/tst-core/src/transport.rs) (trait) | `tst_pipeline::TransportCancel` (re-export of `tst_core::transport::TransportCancel`) |
| Concrete primitive (SRT-shaped) | [`SrtCancelHandle`](../crates/tst-core/src/cancel.rs) (struct) | `tst_pipeline::SrtCancelHandle` (re-export of `tst_core::SrtCancelHandle`); also re-exported as `tst_srt::SrtCancelHandle` |

Pipeline shells return the trait shape:

```rust,ignore
fn cancel_handle(&self) -> Option<Arc<dyn TransportCancel + Send + Sync>>;
```

`Option` because a transport may not support cancellation (a pure
in-memory test mock returns `None`). All real transports — `SrtTransport`,
`SrtRecvTransport`, and `ManagedTransport` decorating either of them —
return `Some`.

`tst-srt`'s `Socket::cancel_handle()` and `Listener::cancel_handle()`
return the concrete `SrtCancelHandle` struct directly (no `Option`); these
are the paths to use when you're working below the pipeline shells.

## Obtaining a handle

From any pipeline shell:

```rust,ignore
use tst_pipeline::{MuxSender, TransportCancel};
use std::sync::Arc;

let mut sender: MuxSender<_> = /* ... */;
let cancel: Option<Arc<dyn TransportCancel + Send + Sync>> = sender.cancel_handle();
let cancel = cancel.expect("real transports always return Some");
```

From `tst-srt` directly (Socket / Listener):

```rust,ignore
use tst_pipeline::SrtCancelHandle;  // re-exported from tst_core::SrtCancelHandle

let socket: tst_srt::Socket = /* ... */;
let cancel: SrtCancelHandle = socket.cancel_handle();
// `SrtCancelHandle: Clone` — the inner state is Arc-shared, so all clones
// fire the closer once across the whole set.
```

## The pattern

The cancel-handle pattern unblocks a thread parked in `send_*` /
`recv_*` from another thread without time-sliced polling:

```rust,no_run
use tst_pipeline::{Sender, SenderConfig, TransportCancel};
use tst_core::transport::{Transport, TransportError};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

# struct Sink;
# impl Transport for Sink {
#     fn send_bytes(&mut self, _: &[u8]) -> Result<(), TransportError> { Ok(()) }
#     fn max_payload(&self) -> usize { 1316 }
#     fn close(&mut self) {}
#     fn is_alive(&self) -> bool { true }
# }
# fn build_real_transport() -> Sink { Sink }
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut sender = Sender::new(build_real_transport(), SenderConfig::default());

    // Snapshot the cancel handle BEFORE we start the send loop. After this
    // point the handle is owned by the cancel thread; the sender keeps
    // running on the main thread until cancel() fires.
    let cancel: Arc<dyn TransportCancel + Send + Sync> = sender
        .cancel_handle()
        .expect("real transports return Some");

    // Cancel thread: in real code this is your signal handler / lifecycle
    // observer / parent-process watchdog.
    thread::spawn(move || {
        thread::sleep(Duration::from_secs(30));
        cancel.cancel();  // wakes the parked send_ts on the main thread
    });

    // Main thread: the parked `send_ts` returns Err(Transport(Broken(_)))
    // once cancel() fires. Loop break is by error, not by polling.
    let pkt = vec![0u8; 188];
    loop {
        match sender.send_ts(&pkt) {
            Ok(()) => continue,
            Err(_e) => break,  // Cancellation surfaces as TransportError::Broken("cancelled")
        }
    }
    Ok(())
}
```

`cancel()` triggers the closer (libsrt's `srt_close` for `SrtTransport`)
exactly once. The parked syscall returns within one libsrt I/O cycle —
in practice 3–10 ms, bounded by the transport's `SRTO_RCVTIMEO` /
`SRTO_SNDTIMEO`. The error surfaces to the caller as
`TransportError::Broken(_)` (the message string is "cancelled" for
shells, or the libsrt error for direct `Socket::send` / `recv`).

## Per-language idiom

Bindings should expose `SrtCancelHandle` as a language-native shutdown
primitive. The shape maps cleanly:

| Language | Idiom |
|----------|-------|
| Rust | `Arc<dyn TransportCancel>` cloned to a worker thread; `cancel.cancel()` |
| Java / Kotlin | `AutoCloseable` cancel-token; `Job.cancel()` analog inside coroutine wrappers |
| Swift | `Task.cancel()` analog; the handle is held by a structured-concurrency parent |
| Python | `threading.Event`-shaped wrapper; `event.set()` triggers the underlying `cancel()` |
| C (`tst-c`) | `tst_cancel_handle_t *` opaque; `tst_cancel_handle_cancel(h)` (deferred to receiver-surface plan) |

The Rust API is the source of truth — every binding crate forwards
`cancel()` to the same idempotent atomic-swap inside `tst-core`.

## Threading guarantees

- `SrtCancelHandle: Send + Sync + Clone`. Stash it in any container, move
  it into any thread, clone it freely — every clone shares the same
  atomic state, so `cancel()` on any clone fires the closer at most
  once across the whole set.
- `Arc<dyn TransportCancel + Send + Sync>` (the shell-level shape) is
  similarly `Send + Sync`; clone the `Arc` to share across threads.
- `cancel()` is idempotent. Calling it twice — or concurrently from
  multiple threads — runs the closer at most once.
- The underlying close (`srt_close` for SRT transports) runs on the
  thread that wins the atomic swap. The closer's return code is
  currently swallowed — see the inline doc on `Socket::close` for
  context.
- `is_cancelled()` (on the concrete `SrtCancelHandle` struct) is advisory;
  the underlying close may not have completed yet on another thread.

## Why this and not `close()`?

`close()` on a shell takes `&mut self`, so it's not callable from
another thread while a `&mut`-borrowing `send_*` is in flight. The
cancel handle gives you a `Send + Sync + Clone` capability that
*doesn't* hold a mutable reference to the shell — the perfect shape
for "another thread / signal handler / FFI consumer needs to wake the
sender".

`MuxSender::close()` internally invokes the cancel handle first
("cancel-then-close" — see plan #18) so that a peer thread parked
inside `send_video` returns promptly, before `close()` proceeds to
flush and tear down. From the outside, `close()` is the right shape
for "stop the shell now" *when the thread doing the close also owns
the shell*; `cancel_handle().cancel()` is the right shape *when it
doesn't*.

## Anti-priorities

`SrtCancelHandle` is the supported shape for sync-blocking shutdown. The
API is intentionally synchronous-blocking; when async lands later as a
separate crate (`tst-srt-async` or feature-gated), it ships
`Future::poll`-shaped cancellation alongside but doesn't deprecate
`SrtCancelHandle` — sync consumers stay on the trait-object pattern
above. See the **Sync vs. async** section in
[`architecture.md`](./architecture.md) for the long form.

## See also

- [`architecture.md`](./architecture.md) — Cross-thread shutdown section.
- [`binding-authors.md`](./binding-authors.md) — Cancel handles for binding authors.
- [`cookbook/operations/31-graceful-shutdown.md`](../cookbook/operations/31-graceful-shutdown.md) — Recipe 31, "Graceful shutdown from a signal handler".
- [`guide-pipeline.md`](./guide-pipeline.md) — Pipeline shell composition (where `cancel_handle()` lives).
