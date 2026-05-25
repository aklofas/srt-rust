# Recipe 0: Send a single TS packet to any `Transport`

> **When to use this:** The simplest possible sender — open a transport, push 188 bytes, drop.

> **Related:**
> - [guides/pipeline.md](/docs/guides/pipeline.md) — the `Transport` trait and the sender shells that compose over it
> - [Recipe 11](11-sender-from-url.md) — real SRT transport from a URL
> - [Recipe 8](08-custom-transport.md) — implementing a custom (non-SRT) transport

The simplest possible sender: open a transport, push 188 bytes, drop.

```rust
use tst_pipeline::{RawSender, RawSenderConfig};
use tst_core::transport::{Transport, TransportError};

// In-memory sink; real callers plug in a `tst_srt::SrtTransport` (recipe 11)
// or any custom Transport (recipe 8).
struct Sink(Vec<u8>);
impl Transport for Sink {
    fn send_bytes(&mut self, b: &[u8]) -> Result<(), TransportError> {
        self.0.extend_from_slice(b);
        Ok(())
    }
    fn max_payload(&self) -> usize { 1316 }
    fn close(&mut self) {}
    fn is_alive(&self) -> bool { true }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut sender = RawSender::new(Sink(Vec::new()), RawSenderConfig::default());
    let mut packet = [0u8; 188];
    packet[0] = 0x47;  // TS sync byte
    sender.send(&packet)?;
    Ok(())
}
```
