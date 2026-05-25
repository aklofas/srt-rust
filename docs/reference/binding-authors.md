# Binding-author starter

This page is the entry point for authors writing language bindings on top of
`ts-transformer`. The Rust API is sync-blocking and generic over a
`Transport` / `RecvTransport`; binding code targets the **dyn-erased**
shape — one concrete type per pipeline shell — for cross-language stability.

## The dyn-erased shape

`tst_pipeline` re-exports six aliases that pin every shell to a boxed
trait object:

| Generic shell | Boxed alias |
|---------------|-------------|
| `MuxSender<T>` | `BoxedMuxSender` |
| `Sender<T>` | `BoxedSender` |
| `RawSender<T>` | `BoxedRawSender` |
| `DemuxReceiver<T>` | `BoxedDemuxReceiver` |
| `Receiver<T>` | `BoxedReceiver` |
| `RawReceiver<T>` | `BoxedRawReceiver` |

The aliases live at both `tst_pipeline::dyn_aliases::*` and the crate root.

Binding code constructs one of these by handing a `Box<dyn Transport>`
(or `Box<dyn RecvTransport>`) to `Xxx::new(transport, config)`:

```rust
use tst_pipeline::{BoxedMuxSender, MuxSender};
use tst_srt::SrtTransport;

let transport: Box<dyn tst_core::Transport> = Box::new(SrtTransport::connect(/* ... */)?);
let mut sender: BoxedMuxSender = MuxSender::new(transport, config)?;
```

## Per-language patterns

### Java / Kotlin (JNI)

Binding shape: opaque handle backed by `jlong` pointing at a `Box<BoxedMuxSender>`.

```kotlin
class MuxSender private constructor(private val handle: Long) : AutoCloseable {
    companion object {
        fun open(url: String, config: MuxerConfig): MuxSender =
            MuxSender(nativeOpen(url, config.toNative()))
    }
    fun sendVideo(payload: ByteArray, ptsTicks: Long) {
        nativeSendVideo(handle, payload, ptsTicks)
    }
    override fun close() { nativeClose(handle) }
}

// Kotlin idiomatic usage: .use { } for AutoCloseable
MuxSender.open(url, config).use { sender ->
    sender.sendVideo(payload, pts)
}
```

### Swift (UniFFI)

Binding shape: opaque struct with `deinit` calling close.

```swift
public class MuxSender {
    private var handle: OpaquePointer?

    public init(url: String, config: MuxerConfig) throws {
        self.handle = try tst_mux_sender_open_url(url, config.toNative())
    }
    public func sendVideo(payload: Data, pts: Int64) throws {
        try tst_mux_sender_send_video(handle, payload, pts)
    }
    deinit { tst_mux_sender_close(handle) }
}

// Swift idiomatic usage: defer { } block
let sender = try MuxSender(url: url, config: config)
defer { /* sender deinit closes automatically */ }
```

### Python (PyO3)

Binding shape: `pyclass` with `__enter__` / `__exit__`.

```python
class MuxSender:
    def __init__(self, url: str, config: MuxerConfig):
        self._handle = _native.mux_sender_open(url, config._native)
    def send_video(self, payload: bytes, pts: int) -> None:
        _native.mux_sender_send_video(self._handle, payload, pts)
    def __enter__(self): return self
    def __exit__(self, exc_type, exc, tb):
        _native.mux_sender_close(self._handle)

# Python idiomatic usage: with-as block
with MuxSender(url, config) as sender:
    sender.send_video(payload, pts)
```

### C (`tst-c` ABI)

Binding shape: opaque handle (`tst_mux_sender_t *`) with explicit
`tst_*_close`. See `crates/tst-c/include/tstrans.h` for the full ABI.

```c
tst_mux_sender_t *sender = NULL;
if (tst_mux_sender_open_url(url, &config, &sender) != TST_OK) { /* error */ }
tst_mux_sender_send_video(sender, payload, payload_len, pts_ticks);
tst_mux_sender_close(sender);
```

### C ABI error-mapping contract

Every C-ABI error code surfaced through `tst_get_last_error()` and the
direct negative-return-value contract derives from one of two
explicit-coverage paths in `crates/tst-c/src/error.rs`:

**Shell-entry path (the common case).** Every C entry point that owns
a shell handle (`tst_mux_sender_*`, `tst_ts_sender_*`,
`tst_raw_sender_*`, `tst_demux_receiver_*`, `tst_ts_receiver_*`,
`tst_raw_receiver_*`) routes errors through one generic helper:

```rust
record_shell_error<E: ShellError>(e: &E) -> i32
// 1. e.kind() -> ShellErrorKind
// 2. tst_error_from_kind(kind) -> TstError
// 3. set_last_error(code, &e.to_string())
// 4. return negative code
```

Two CI ratchets guard this path against silent regressions:

- `scripts/check-shell-error-kind-coverage.sh` — fails if a future
  `ShellErrorKind` variant is added without an explicit arm in
  `tst_error_from_kind` (before the `#[non_exhaustive]` wildcard).
- `scripts/check-pipeline-kind-classification.sh` — fails if a future
  variant of `MuxError`, `TransportError`, `DemuxError`, or
  `TsFramingError` is added without an explicit arm in the
  corresponding `kind_from_*` helper in
  `crates/tst-pipeline/src/shell_error.rs`.

**Raw-mapper path (standalone-muxer + open helpers).** Two C-ABI paths
surface upstream errors before any shell wraps them and still go
through dedicated per-variant tables:

- `record_mux_error(&MuxError)` — used by `tst_muxer_*` (the
  standalone muxer, no transport).
- `record_transport_error(&TransportError)` — used by `tst_*_open_url`
  / `tst_*_open_addr` / `tst_*_listen_*` for connect/listen failures
  surfaced before a shell exists.

One CI ratchet guards this path:

- `scripts/check-raw-c-mapper-coverage.sh` — fails if a future
  `MuxError` or `TransportError` variant is added without an explicit
  arm in the corresponding `record_*_error` function.

Each path's wildcard `_ => ...` arm exists only to satisfy Rust's
`#[non_exhaustive]` requirement and is unreachable when the
corresponding ratchet is green. Binding authors can therefore assume
that every documented `TstError` code maps to a specific upstream
condition; no upstream variant silently degrades to `TST_E_INTERNAL`,
`TST_E_INVALID_CONFIG`, or `TST_E_TRANSPORT` without an explicit
choice by the tst-c maintainers.

If you encounter a `tst_get_last_error_str()` value beginning with
`"unhandled <Enum> variant: ..."`, that means one of the three
ratchets was bypassed or failed; please file an issue with the
variant name from the last-error string.

### Transient vs persistent error codes

Two negative `TST_E_*` codes — `TST_E_NOT_AVAILABLE` (-13) and
`TST_E_NOT_FOUND` (-14) — share a "the data you asked for isn't here"
shape but differ on whether the caller should retry:

| Code | Contract | When | Bindings should... |
|------|----------|------|---------------------|
| `TST_E_NOT_AVAILABLE` (-13) | **Transient** — the next call may succeed. | A `tst_managed_*` handle's underlying transport is reconnecting; stats / socket_stats are momentarily inaccessible. | Surface as a "retry later" signal. Example: Java/Kotlin → return `Optional.empty()` and let the caller poll; Swift → return `nil`; Python → return `None`. No user-visible exception. |
| `TST_E_NOT_FOUND` (-14) | **Persistent** — the next call with the same key will return the same error. | A per-PID accessor (e.g., `tst_*_get_stream_codec_stats`) was asked about a PID the demuxer has never seen on this handle. | Surface as a fail-fast lookup miss. Example: Java/Kotlin → throw `NoSuchElementException`; Swift → throw a typed error; Python → raise `KeyError`. |

`TST_E_INVALID_USAGE` (-9) is the third sibling — used when the handle
is in a fundamentally wrong state for the call (e.g., `tst_*_send_video`
on a handle that's already been closed). Distinct from both above
because it's a programmer bug, not a runtime data-availability question.

When designing your binding's retry policy, key the decision off the
TST_E code, not the message string (the message is a debug aid and is
not part of the stable contract — see "C ABI error-mapping contract"
above).

## Cancel handles

Every long-lived shell exposes `cancel_handle()` returning a `SrtCancelHandle`
that's `Send + Sync` and one-shot. Bindings should expose this as a
language-native shutdown primitive (e.g. Kotlin `Job.cancel()` analog,
Swift `Task.cancel()` analog, Python `threading.Event`-shaped). See
`docs/srt-cancel-handle.md` for the full pattern.

## Builder shape

Every public builder (`MuxerConfigBuilder`, `MuxerProgramConfigBuilder`,
`SocketBuilder`, `ListenerBuilder`) uses `&mut self -> &mut Self`. This
shape translates directly to:

| Language | Builder usage |
|----------|---------------|
| Kotlin | `MuxerConfigBuilder().apply { addProgram(...); }.build()` |
| Java | `new MuxerConfigBuilder().addProgram(...).build()` |
| Swift | `var b = MuxerConfigBuilder(); b.addProgram(...); b.build()` |
| Python | `b = MuxerConfigBuilder(); b.add_program(...); b.build()` |
| C | `tst_muxer_config_builder_t *b = ...; tst_muxer_config_builder_add_program(b, ...);` |

## Threading model

The threading guarantees split into two tiers — runtime handles vs.
opaque config builders. Read both before exposing tst-c objects as
thread-safe in your binding.

**Runtime handles** (the shells produced by `tst_*_open` /
`tst_*_open_listener`) are guarded by `Handle<T> = Mutex<...>` inside
`tst-c`. Data-path entry points (`_send_*`, `_recv_*`, `_pull`,
`_get_stats`, `_push_*`, etc.) acquire that lock internally. Bindings
may safely call data-path functions on the same handle from any
thread — concurrent calls serialize through the inner mutex. Cancel
entry points (`_cancel`) intentionally do NOT acquire the mutex
(they're side-channels for unblocking a parked I/O thread), so they
can be invoked concurrently with data-path calls without
deadlocking. Each runtime handle is `Send` (ownership can transfer
between threads) and effectively `Sync` for data-path use through the
inner mutex.

**Opaque config builders** (`TstMuxConfig`, `TstDemuxConfig`,
`TstSenderConfig`, `TstRawSenderConfig`, `TstReconnectPolicy`) are
raw `Box<T>` values with unsynchronized mutable fields. Their setters
(`tst_mux_config_add_program`, `tst_demux_config_set_strict_mode`,
etc.) mutate through `&mut T` with NO internal locking. Bindings MUST
either:

- confine each builder pointer to a single thread for its entire
  lifetime (recommended — builders are short-lived by design), OR
- add a language-side lock (e.g. a Kotlin `synchronized`, a Swift
  `NSLock`, a Python `threading.Lock`) around every setter call on a
  shared builder.

Concurrent calls to two setters on the same builder pointer from
different threads without a language-side lock are a data race and
undefined behavior.

**Drop ordering.** Across two runtime shells that share state (e.g. a
sender + receiver pair that both reference the same underlying SRT
socket), `Drop` ordering is undefined. Consumers should drop senders
before receivers when both share state.

## Versioning

The Rust API uses pre-1.0 SemVer (`0.x.y`). The C ABI uses its own
three-tier scheme exposed through `tstrans.h`:

- `TST_ABI_VERSION_MAJOR` — incremented on any source- or binary-
  incompatible change to the ABI shape. **0** today.
- `TST_ABI_VERSION_MINOR` — incremented on additive, source-compatible
  changes (new event kinds, new C entry points, new error codes).
  **5** today. History (additive bumps only — major stays at 0 pre-1.0):
    - `1` (plan #62): receiver-surface initial drop.
    - `2` (validate-1 Phase 2 wrap-up): `ManagedDemuxReceiver` wired into
      `tst-c`; `TST_EVENT_RECONNECT_DISCONTINUITY = 6` added; TS-bytes
      raw-receiver pull-loop hardening + F2 C-ABI shape additions.
    - `3` (AU cell reassembly, 2026-05-24): `TstMultiCellAuReason` +
      `multi_cell_au_reason` field on `TstEventNonConformant`.
    - `4` (AU cell CFI tolerance, 2026-05-24):
      `TstNonConformantCode::CfiTolerated` (= 32) + `TstCellFragmentIndication`
      enum + `tst_demux_config_set_cfi_tolerance` setter. The new variant
      reuses the existing `cc_expected` + `cc_observed` field carriers
      to surface `observed_cfi` + `treated_as` without growing the struct.
    - `5` (plan #96 demuxer-config parity, 2026-05-25):
      new C entry points `tst_demux_config_set_av1_carriage`,
      `tst_demux_config_set_au_cell_cap_per_pid`, and
      `tst_demux_config_set_lenient_psi_reassembly`, plus the
      `TstAv1CarriageMode` enum. Bridges Rust-only `DemuxerConfig`
      knobs through the C builder.
- `TST_ABI_VERSION_PATCH` — incremented on internal fixes that
  preserve both shape and behaviour.

Bindings should compile-time-assert the minor they require:

```c
#if TST_ABI_VERSION_MAJOR != 0 || TST_ABI_VERSION_MINOR < 5
#  error "this binding requires tst-c ABI ≥ 0.5"
#endif
```

`srt-jni` and `srt-uniffi` track the C ABI for binary stability and the
Rust crate for feature parity.

## Thread-local last-error reset

`tst_clear_last_error()` (added in ABI 0.1) clears the thread-local
`(code, message)` slot read by `tst_get_last_error()` /
`tst_get_last_error_str()`. Bindings should call it at the start of
every public entry point that may return success but doesn't otherwise
overwrite the slot — without the clear, callers polling the error
string after a successful call see stale data from the previous call.

## Receiver-side reconnect (ABI 0.2)

`tst_managed_demux_receiver_*` mirrors the Rust `ManagedDemuxReceiver`
shell. When the underlying transport reconnects, the next
`tst_managed_demux_receiver_recv_event` emits a
`TST_EVENT_RECONNECT_DISCONTINUITY` event (kind = 6) so the language
binding can mark a hard discontinuity in any downstream state
(timestamp resets, PSI cache invalidation, decoder reset, etc.) instead
of guessing from the byte stream. The demuxer's `reset_sync()` is
called transparently before the next packet hits the syncer; reassembly
tables (PAT/PMT, per-PID CC, last PTS) are preserved across the
reconnect.
