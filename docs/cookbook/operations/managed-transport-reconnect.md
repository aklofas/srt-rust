# Survive a flaky transport with reconnect + gap buffer

> **When to use this:** The wire is lossy — radio links, NAT timeouts, listener restarts.

> **Related:**
> - [guides/pipeline.md](/docs/guides/pipeline.md) — `ManagedTransport`, `ReconnectPolicy`, and gap-buffer behavior
> - [Example: `managed_reconnect`](/examples/operations/managed_reconnect.rs)

Reach for this when the wire is lossy — radio links, NAT timeouts, listener restarts. `ManagedTransport<T>` decorates any `Transport` impl with a reconnect loop and a bounded gap buffer; the wrapped sender shell sees a `Transport` that occasionally pauses but never fails on transient breakage.

The factory closure rebuilds the inner transport on demand. `ReconnectPolicy` controls retries, backoff, and gap-buffer overflow behaviour.

```rust,no_run
use tst_core::mpegts::mux::MuxerConfig;
use tst_pipeline::{
    BackoffStrategy, ManagedTransport, MuxSender, OverflowPolicy, ReconnectPolicy, TransportError,
};
use tst_srt::{SocketBuilder, SrtTransport};
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let factory = || -> Result<SrtTransport, TransportError> {
        // Bind-then-chain: mutators borrow, terminal `connect` borrows.
        let mut sb = SocketBuilder::new();
        sb.latency(Duration::from_millis(120));
        let socket = sb
            .connect("127.0.0.1:9000")
            .map_err(|e| TransportError::Broken(format!("connect failed: {e}")))?;
        Ok(SrtTransport::new(socket))
    };
    let initial = factory()?;
    let policy = ReconnectPolicy {
        max_attempts: Some(20),
        backoff: BackoffStrategy::Exponential {
            base: Duration::from_millis(100),
            max: Duration::from_secs(10),
        },
        gap_buffer_capacity: 256,
        overflow_policy: OverflowPolicy::DropOldest,
    };
    let managed = ManagedTransport::new(initial, factory, policy);
    let _sender = MuxSender::new(managed, MuxerConfig::default())?;
    Ok(())
}
```

Runnable: [examples/operations/managed_reconnect.rs](/examples/operations/managed_reconnect.rs).
