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

Every variant of the upstream pipeline error enums — `tst_core::error::MuxError`,
`tst_core::transport::TransportError`, `tst_pipeline::MuxSenderError`, and
`tst_pipeline::sender::SenderError` — is explicitly mapped to a `TstError`
code in `crates/tst-c/src/error.rs` via the corresponding `record_*_error`
function. Each function's wildcard `_ => ...` arm exists only to satisfy
Rust's `#[non_exhaustive]` requirement; it is unreachable in normal use.

The CI ratchet `scripts/check-tst-c-error-coverage.sh` enforces this
contract: when an upstream variant is added, the script fails until the
variant is explicitly handled in the relevant `record_*_error` function
body before the wildcard. Binding authors can therefore assume that every
documented `TstError` code maps to a specific upstream condition, and that
no upstream variant silently degrades to `TST_E_INTERNAL`,
`TST_E_INVALID_CONFIG`, or `TST_E_TRANSPORT` without an explicit choice by
the tst-c maintainers.

If you encounter a `tst_get_last_error_str()` value beginning with
`"unhandled <Enum> variant: ..."`, that means the ratchet was bypassed or
failed; please file an issue with the variant name from the last-error
string.

## Cancel handles

Every long-lived shell exposes `cancel_handle()` returning a `CancelHandle`
that's `Send + Sync` and one-shot. Bindings should expose this as a
language-native shutdown primitive (e.g. Kotlin `Job.cancel()` analog,
Swift `Task.cancel()` analog, Python `threading.Event`-shaped). See
`docs/cancel-handle.md` for the full pattern.

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

`Send + Sync` everywhere. Bindings can move shells across threads freely.
The exception: the `Drop` ordering across two `Send` shells is undefined,
so consumers should drop senders before receivers when both share state.

## Versioning

The Rust API uses pre-1.0 SemVer (`0.x.y`). The C ABI uses its own
versioning scheme keyed to `tstrans.h` symbol stability. JNI / UniFFI /
PyO3 bindings track the C ABI for binary stability and the Rust crate
for feature parity.
