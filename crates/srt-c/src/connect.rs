//! Shared `connect_srt` helper used by all six `srtc_*_open` calls.
//!
//! Each `_open` builds a fresh `SocketConfig` (typically from a parsed
//! URL overlay) and calls `connect_srt(host, port, &cfg)` to produce
//! an `SrtTransport`. The plain senders use the result directly; the
//! managed senders capture (host, port, cfg) in the reconnect closure.

use srt_core::pipeline::{SrtTransport, TransportError};
use srt_core::srt::{Socket, SocketConfig};

/// Build a fresh `SrtTransport` connected to `host:port` using the
/// provided socket config (passphrase, latency, etc. set as captured
/// from the URL overlay). Returns `TransportError::Broken` on connect
/// failure for unified surfacing through `record_transport_error`.
// Callers wired in Task 13/14.
#[allow(dead_code)]
pub(crate) fn connect_srt(
    host: &str,
    port: u16,
    cfg: &SocketConfig,
) -> Result<SrtTransport, TransportError> {
    let socket = Socket::connect_with(cfg, format!("{host}:{port}").as_str())
        .map_err(|e| TransportError::Broken(format!("connect: {e}")))?;
    Ok(SrtTransport::new(socket))
}
