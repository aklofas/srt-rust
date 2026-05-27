//! Internal hyper 1.x HTTP server (serves /playlist.m3u8 + /segment_*.ts).
//!
//! The server runs on a private tokio Runtime owned by [`HlsPublisher`].
//! Callers see only the sync `Publisher` API.

use std::convert::Infallible;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::header::{AUTHORIZATION, CONTENT_TYPE, WWW_AUTHENTICATE};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio::runtime::Runtime;
use tokio_util::sync::CancellationToken;

use crate::hls::auth::check_basic_auth;
use crate::hls::error::HlsError;
use crate::hls::playlist;
use crate::hls::publisher::State;

/// Handle owned by the publisher; cancels + joins the runtime on drop.
pub(crate) struct ServerHandle {
    pub(crate) cancel: CancellationToken,
    pub(crate) runtime: Option<Runtime>,
    pub(crate) local_addr: std::net::SocketAddr,
}

impl ServerHandle {
    pub(crate) fn start(
        state: Arc<Mutex<State>>,
        bind: std::net::SocketAddr,
        basic_auth: Option<(String, String)>,
        #[cfg(feature = "tls")] tls_config: Option<Arc<rustls::ServerConfig>>,
    ) -> Result<Self, HlsError> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("tst-tcp-hls-rt")
            .enable_all()
            .build()
            .map_err(HlsError::Io)?;

        let cancel = CancellationToken::new();
        let cancel_for_task = cancel.clone();
        let state_for_task = state.clone();

        #[cfg(feature = "tls")]
        let acceptor = tls_config.map(tokio_rustls::TlsAcceptor::from);

        let (listener, local_addr) = runtime
            .block_on(async {
                let l = TcpListener::bind(bind).await?;
                let a = l.local_addr()?;
                Ok::<_, std::io::Error>((l, a))
            })
            .map_err(|e| HlsError::BindFailed(e.to_string()))?;

        runtime.spawn(async move {
            loop {
                tokio::select! {
                    _ = cancel_for_task.cancelled() => break,
                    accepted = listener.accept() => {
                        let (sock, _peer) = match accepted {
                            Ok(p) => p,
                            Err(_) => continue,
                        };
                        let state = state_for_task.clone();
                        let auth = basic_auth.clone();
                        let conn_cancel = cancel_for_task.clone();
                        #[cfg(feature = "tls")]
                        let acceptor_clone = acceptor.clone();

                        tokio::spawn(async move {
                            #[cfg(feature = "tls")]
                            if let Some(acc) = &acceptor_clone {
                                match acc.accept(sock).await {
                                    Ok(tls_sock) => {
                                        let io = TokioIo::new(tls_sock);
                                        let svc = service_fn(move |req| {
                                            let state = state.clone();
                                            let auth = auth.clone();
                                            async move { Ok::<_, Infallible>(serve(req, state, auth).await) }
                                        });
                                        let _ = tokio::select! {
                                            _ = conn_cancel.cancelled() => Ok(()),
                                            r = http1::Builder::new().serve_connection(io, svc) => r,
                                        };
                                    }
                                    Err(_) => {} // bad TLS handshake — drop
                                }
                                return;
                            }
                            // Plain HTTP path (no TLS feature OR TLS feature but no config)
                            let io = TokioIo::new(sock);
                            let svc = service_fn(move |req| {
                                let state = state.clone();
                                let auth = auth.clone();
                                async move { Ok::<_, Infallible>(serve(req, state, auth).await) }
                            });
                            let _ = tokio::select! {
                                _ = conn_cancel.cancelled() => Ok(()),
                                r = http1::Builder::new().serve_connection(io, svc) => r,
                            };
                        });
                    }
                }
            }
        });

        Ok(Self {
            cancel,
            runtime: Some(runtime),
            local_addr,
        })
    }

    pub(crate) fn local_addr(&self) -> std::net::SocketAddr {
        self.local_addr
    }

    /// Cancel + wait for the runtime to drain.
    pub(crate) fn shutdown(mut self) {
        self.cancel.cancel();
        if let Some(rt) = self.runtime.take() {
            rt.shutdown_timeout(std::time::Duration::from_secs(2));
        }
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Some(rt) = self.runtime.take() {
            rt.shutdown_timeout(std::time::Duration::from_secs(2));
        }
    }
}

/// Per-request handler.
async fn serve(
    req: Request<Incoming>,
    state: Arc<Mutex<State>>,
    basic_auth: Option<(String, String)>,
) -> Response<Full<Bytes>> {
    // Auth check (if configured).
    if let Some((user, pass)) = &basic_auth {
        let header = req
            .headers()
            .get(AUTHORIZATION)
            .and_then(|v| v.to_str().ok());
        if !check_basic_auth(user, pass, header) {
            return Response::builder()
                .status(StatusCode::UNAUTHORIZED)
                .header(WWW_AUTHENTICATE, r#"Basic realm="tst-tcp HLS""#)
                .body(Full::new(Bytes::from_static(b"Unauthorized")))
                .unwrap();
        }
    }

    let path = req.uri().path();

    if path == "/playlist.m3u8" {
        let pl = {
            let s = state.lock().expect("HlsPublisher poisoned");
            playlist::render(&s.segmenter, false)
        };
        return Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, "application/vnd.apple.mpegurl")
            .body(Full::new(Bytes::from(pl)))
            .unwrap();
    }

    if let Some(filename) = path.strip_prefix('/') {
        if filename.starts_with("segment_") && filename.ends_with(".ts") {
            let bytes = {
                let s = state.lock().expect("HlsPublisher poisoned");
                s.segmenter.read_segment(filename)
            };
            return match bytes {
                Ok(b) => Response::builder()
                    .status(StatusCode::OK)
                    .header(CONTENT_TYPE, "video/mp2t")
                    .body(Full::new(Bytes::from(b)))
                    .unwrap(),
                Err(_) => Response::builder()
                    .status(StatusCode::NOT_FOUND)
                    .body(Full::new(Bytes::from_static(b"Not Found")))
                    .unwrap(),
            };
        }
    }

    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body(Full::new(Bytes::from_static(b"Not Found")))
        .unwrap()
}
