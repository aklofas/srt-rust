//! Shared `connect_srt` helper used by all six `tst_*_open` calls.
//!
//! Each `_open` builds a fresh `SocketConfig` (typically from a parsed
//! URL overlay) and calls `connect_srt(host, port, &cfg)` to produce
//! an `SrtTransport`. The plain senders use the result directly; the
//! managed senders capture (host, port, cfg) in the reconnect closure.

use std::time::Duration;
use tst_pipeline::TransportError;
use tst_srt::SrtTransport;
use tst_srt::options::Role;
use tst_srt::{Socket, SocketConfig};

/// MuxSender-pipeline default for `SRTO_CONNTIMEO`. libsrt's default is 3s,
/// which is too short for the radio-link domain this library targets
/// (LOS-over-terrain interruptions, antenna repointing, radio warm-up).
const SENDER_DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// MuxSender-pipeline default for `SRTO_LINGER`. libsrt's default is 180s
/// (3 minutes), which lets `Socket::Drop` block for that long when the
/// peer doesn't ACK pending sends — particularly painful inside a
/// `ManagedTransport` reconnect cycle. 5s is long enough to drain a
/// small backlog under healthy conditions, short enough to never stall
/// reconnect noticeably.
const SENDER_DEFAULT_LINGER: Duration = Duration::from_secs(5);

/// Build a fresh `SrtTransport` connected to `host:port` using the
/// provided socket config (passphrase, latency, etc. set as captured
/// from the URL overlay). Returns `TransportError::Broken` on connect
/// failure for unified surfacing through `record_transport_error`.
///
/// Applies sender-pipeline defaults to the config in place when the
/// caller hasn't set them (currently: `connect_timeout = 15s`,
/// `linger = 5s`). User-set values are preserved. Always sets
/// `role = Role::MuxSender` (drives `SRTO_SENDER=1` for HSv4-peer
/// latency-negotiation compatibility — the canonical "default sender
/// connect path"; harmless under HSv5).
pub(crate) fn connect_srt(
    host: &str,
    port: u16,
    cfg: &SocketConfig,
) -> Result<SrtTransport, TransportError> {
    let mut cfg = cfg.clone();
    if cfg.connect_timeout.is_none() {
        cfg.connect_timeout = Some(SENDER_DEFAULT_CONNECT_TIMEOUT);
    }
    if cfg.linger.is_none() {
        cfg.linger = Some(SENDER_DEFAULT_LINGER);
    }
    cfg.role = Role::MuxSender;
    let socket = Socket::connect_with(&cfg, format!("{host}:{port}").as_str())
        .map_err(|e| TransportError::Broken(format!("connect: {e}")))?;
    Ok(SrtTransport::new(socket))
}
