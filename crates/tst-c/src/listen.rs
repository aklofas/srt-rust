//! Shared `listen_srt` helper used by all `tst_*_open_listener` calls
//! (and by URL-driven `_open` calls when `mode=listener` is detected).
//!
//! Each `_open_listener` builds a `ListenerConfig` (typically from a
//! parsed URL overlay), binds the local address, blocks on `accept()`
//! until the first peer completes the SRT handshake, and returns the
//! accepted `SrtTransport`. Subsequent peer connections on the same
//! port are not handled — single-accept matches the connection-oriented
//! shape of every other entry point in `tst-c`.

use tst_pipeline::TransportError;
use tst_srt::SrtTransport;
use tst_srt::config::ListenerConfig;
use tst_srt::Listener;

/// Build a `Listener` bound to `host:port`, block on `accept()`, and
/// return the accepted `SrtTransport`. The listener is dropped before
/// return — single-accept semantics.
///
/// `host` may be empty (binds wildcard `0.0.0.0`) or a specific bind
/// address. `cfg` is the `ListenerConfig` (receiver timeouts and other
/// listen-side settings are applied here). Accepted sockets inherit the
/// listen-side `send_timeout` and `recv_timeout` per libsrt's option
/// inheritance mechanism.
///
/// Returns `TransportError::Broken` on bind or accept failure for
/// unified surfacing through `record_transport_error`.
pub(crate) fn listen_srt(
    host: &str,
    port: u16,
    cfg: &ListenerConfig,
) -> Result<SrtTransport, TransportError> {
    let bind_host = if host.is_empty() { "0.0.0.0" } else { host };
    let addr = format!("{bind_host}:{port}");
    let mut listener = Listener::bind_with(cfg, addr.as_str())
        .map_err(|e| TransportError::Broken(format!("bind: {e}")))?;
    let (socket, _peer) = listener
        .accept()
        .map_err(|e| TransportError::Broken(format!("accept: {e}")))?;
    Ok(SrtTransport::new(socket))
}
