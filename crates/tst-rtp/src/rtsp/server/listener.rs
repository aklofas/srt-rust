//! Server accept loop. Binds a tokio `TcpListener` to the configured
//! bind URL, runs an async accept loop, and spawns
//! `session::handle_connection` per accepted client. For `rtsps://`
//! binds, the accepted TCP stream is handed to
//! `tls::TlsServerConfig::accept` (lands at Task 11) before being
//! passed to the per-session task.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use tokio::net::TcpListener;

use crate::error::RtspServerError;
use crate::rtsp::server::ServerState;
use crate::url::RtspScheme;

/// Run the accept loop until cancellation or unrecoverable error.
///
/// Sets `state.local_addr` once the listener binds (so `RtspServer::start`'s
/// spin-wait can return). Continues accepting until either the hard
/// cancel handle is flipped or the graceful cancellation token fires.
pub(crate) async fn run_listener(state: Arc<ServerState>) -> Result<(), RtspServerError> {
    // Bind. `state.builder.bind_url` is guaranteed to be IP-literal at
    // builder time (validate_for_server_bind enforced it).
    let bind_addr = format!(
        "{}:{}",
        state.builder.bind_url.host, state.builder.bind_url.port
    );
    let listener = TcpListener::bind(&bind_addr).await.map_err(|e| {
        if e.kind() == std::io::ErrorKind::AddrInUse {
            RtspServerError::BindAddrInUse
        } else {
            RtspServerError::Io(e.kind())
        }
    })?;
    // Resolve the actual bound address — important when bind port was 0
    // (kernel picks). RtspServer::start() spin-waits on local_addr.
    let local = listener
        .local_addr()
        .map_err(|e| RtspServerError::Io(e.kind()))?;
    *state.local_addr.lock().unwrap() = Some(local);
    tracing::info!(target: "tst_rtp::server", "RTSP server listening at {local}");

    // For `rtsps://` builds, load the TLS server config once at bind
    // time (cert + key from RtspServerBuilder::tls_cert paths). This is
    // feature-gated; without `tls` feature, the rtsps scheme path is
    // unreachable (Cargo.toml gating + builder's tls_cert method are
    // both feature-gated).
    let is_tls = matches!(state.builder.bind_url.scheme(), RtspScheme::Rtsps);
    #[cfg(feature = "tls")]
    let tls_config = if is_tls {
        let cert = state.builder.tls_cert_path.as_ref().ok_or_else(|| {
            RtspServerError::Tls("rtsps:// bind requires tls_cert() builder call".into())
        })?;
        let key = state.builder.tls_key_path.as_ref().ok_or_else(|| {
            RtspServerError::Tls("rtsps:// bind requires tls_cert() builder call".into())
        })?;
        Some(crate::rtsp::server::tls::TlsServerConfig::load(cert, key)?)
    } else {
        None
    };
    #[cfg(not(feature = "tls"))]
    if is_tls {
        return Err(RtspServerError::Tls(
            "rtsps:// bind requires the 'tls' cargo feature".into(),
        ));
    }

    loop {
        if state.hard_cancel.is_canceled() {
            tracing::info!(target: "tst_rtp::server", "hard cancel observed; listener exiting");
            return Ok(());
        }
        tokio::select! {
            accept_res = listener.accept() => {
                match accept_res {
                    Ok((tcp, peer)) => {
                        // Max-sessions guard — reserve the slot ATOMICALLY here,
                        // in the single-threaded accept loop, BEFORE spawning the
                        // per-session task. A plain load-then-check raced an
                        // unauthenticated connection burst: the loop accepts +
                        // spawns faster than the spawned tasks get polled to
                        // increment, so every accept read the same low count and
                        // all passed the check, blowing past the cap. `fetch_add`
                        // makes the count accurate at accept time; a burst now
                        // correctly observes the incremented value. On over-cap we
                        // immediately `fetch_sub` the reservation and refuse (drop
                        // the TCP + continue). On success the reservation is owned
                        // by a `SessionSlot` RAII guard moved into the spawned task,
                        // whose `Drop` releases the slot on EVERY exit path
                        // (including a TLS-handshake failure that returns before the
                        // session loop runs — the leak the old in-session
                        // `fetch_sub` couldn't cover).
                        let prev = state.active_sessions.fetch_add(1, Ordering::Relaxed);
                        if prev >= state.builder.max_sessions {
                            state.active_sessions.fetch_sub(1, Ordering::Relaxed);
                            tracing::warn!(
                                target: "tst_rtp::server",
                                "max sessions ({}) reached; refusing {peer}",
                                state.builder.max_sessions
                            );
                            drop(tcp);
                            continue;
                        }
                        let st = state.clone();
                        let slot = crate::rtsp::server::session::SessionSlot::new(state.clone());
                        #[cfg(feature = "tls")]
                        {
                            let cfg = tls_config.clone();
                            if let Some(cfg) = cfg {
                                tokio::spawn(async move {
                                    // `slot` is moved into this task so the reserved
                                    // slot is released (its `Drop` fires) on BOTH the
                                    // handshake-failure path and the normal session
                                    // path.
                                    let _slot = slot;
                                    match cfg.accept(tcp).await {
                                        Ok(tls_stream) => {
                                            if let Err(e) = crate::rtsp::server::session::handle_connection_tls(st, tls_stream, peer).await {
                                                tracing::warn!(
                                                    target: "tst_rtp::server",
                                                    peer = %peer, error = ?e,
                                                    "tls session ended with error"
                                                );
                                            }
                                        }
                                        Err(e) => {
                                            tracing::warn!(
                                                target: "tst_rtp::server",
                                                peer = %peer, error = ?e,
                                                "TLS handshake failed"
                                            );
                                        }
                                    }
                                });
                                continue;
                            }
                        }
                        // Plain TCP path (always available regardless of `tls` feature):
                        tokio::spawn(async move {
                            if let Err(e) = crate::rtsp::server::session::handle_connection(st, tcp, peer, slot).await {
                                tracing::warn!(
                                    target: "tst_rtp::server",
                                    peer = %peer, error = ?e,
                                    "session ended with error"
                                );
                            }
                        });
                    }
                    Err(e) => {
                        tracing::warn!(
                            target: "tst_rtp::server",
                            error = %e,
                            "accept failed; continuing"
                        );
                    }
                }
            }
            _ = state.cancel_token.cancelled() => {
                tracing::info!(target: "tst_rtp::server", "graceful cancel observed; listener exiting");
                return Ok(());
            }
            _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {
                // Wake to check hard_cancel flag.
            }
        }
    }
}
