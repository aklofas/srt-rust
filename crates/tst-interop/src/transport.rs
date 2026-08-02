//! URL → transport construction, dispatched by scheme.
//!
//! [`make_send`]/[`make_recv`] switch on the URL's scheme prefix
//! (`srt://`, `udp://`, `tcp://`/`tcps://`, `rist://`) and hand off to
//! that transport crate's own `*Url`/`*Transport`/`*RecvTransport`
//! constructors — this module does not re-implement URL parsing or
//! option validation; every per-crate URL type already owns its query
//! parameters (passphrase, latency, `?listen=1`, `?mode=`, the `@`
//! recv-bind convention, etc.). The construction call form for each
//! scheme is lifted from the matching example under `examples/`: udp
//! from `sending/send_udp.rs` + `receiving/recv_udp.rs`, tcp/tcps from
//! `sending/send_tcp.rs` + `receiving/recv_tcp.rs`, rist from
//! `sending/send_rist.rs` + `receiving/recv_rist.rs`, and the SRT
//! URL-overlay pattern [`srt_socket`] follows from
//! `sending/sender_from_url.rs`.
//!
//! # Byte-transparency tee
//!
//! [`Teeing`] wraps a constructed transport and hashes+counts every byte
//! it forwards into a shared tap ([`tee_tally`] reads the tally back
//! once the pipeline shell that owns the `Teeing` has been dropped).
//! `send.rs`/`recv.rs` use this for `CellMetrics::bytes`/`stream_sha256`
//! — the exact bytes that crossed the wire, independent of whatever the
//! muxer/demuxer parsed from them.
//!
//! # Bounded receive
//!
//! Every scheme's recv-side transport here is built so `recv_bytes`
//! periodically returns `TransportError::Backpressure` instead of
//! blocking forever — `recv::recv_over_transport` drives a wall-clock
//! deadline loop that needs that periodic control back (see that
//! function's doc comment). `tst-rist`'s receiver already does this
//! natively (librist's own ~100ms poll, see `examples/receiving/
//! recv_rist.rs`); SRT gets a modest default `SRTO_RCVTIMEO` here (the
//! URL's `x-recvtimeout` still wins if set); UDP gets [`BoundedUdpRecv`],
//! which drives `UdpRecvTransport`'s own `recv_timeout` escape hatch
//! (its trait-level `recv_bytes` blocks indefinitely by design — see
//! that type's own rustdoc). **TCP has no equivalent knob anywhere in
//! its public API** (no read-timeout field on `tst_tcp::config::
//! SocketConfig`), so a TCP recv cell that never sees more data and
//! never closes/breaks the connection would hang the deadline loop.
//! Not exercised by this crate's loopback tests today (udp + srt only);
//! revisit if a future TCP interop cell needs the same bound.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use sha2::{Digest, Sha256};
use tst_core::transport::{RecvTransport, SocketStats, Transport, TransportCancel, TransportError};

/// Default `SRTO_RCVTIMEO` applied to every SRT socket/listener this
/// module builds, unless the URL's `x-recvtimeout` overrides it.
/// Without this, SRT's `recv_bytes` blocks forever (see
/// `examples/receiving/srt_listener_to_file.rs`'s doc comment on
/// `RecvError::TimedOut`) and `recv::recv_over_transport`'s deadline
/// loop could never regain control to check the wall clock.
const DEFAULT_SRT_RECV_TIMEOUT: Duration = Duration::from_millis(200);

/// Default `SRTO_CONNTIMEO` applied to every SRT caller socket this
/// module builds, unless the URL's `conntimeo`/`connect_timeout`
/// overrides it. libsrt's own default is 3s; this crate's send/recv
/// subcommands run short, finite test cells, so failing a dead connect
/// attempt faster (still generous for a real link) keeps a
/// retry-on-connect caller (see `tests/loopback.rs`) fast without
/// giving up too eagerly on a genuinely slow handshake.
const DEFAULT_SRT_CONNECT_TIMEOUT: Duration = Duration::from_secs(2);

/// Poll granularity for [`BoundedUdpRecv`] — see the module doc's
/// "Bounded receive" section.
const UDP_RECV_POLL: Duration = Duration::from_millis(200);

/// Build the sending (caller/push) half of `url`'s scheme.
pub fn make_send(url: &str) -> Result<Box<dyn Transport>, String> {
    match scheme_of(url)? {
        "udp" => tst_udp::UdpTransport::connect(url)
            .map(|t| Box::new(t) as Box<dyn Transport>)
            .map_err(|e| format!("udp connect {url}: {e}")),
        "tcp" | "tcps" => tcp_transport(url).map(|t| Box::new(t) as Box<dyn Transport>),
        "rist" => tst_rist::RistTransport::connect(url)
            .map(|t| Box::new(t) as Box<dyn Transport>)
            .map_err(|e| format!("rist connect {url}: {e}")),
        "srt" => {
            srt_socket(url).map(|s| Box::new(tst_srt::SrtTransport::new(s)) as Box<dyn Transport>)
        }
        other => Err(format!("unsupported scheme for send: {other}://")),
    }
}

/// Build the receiving (listener/pull) half of `url`'s scheme.
pub fn make_recv(url: &str) -> Result<Box<dyn RecvTransport>, String> {
    match scheme_of(url)? {
        "udp" => tst_udp::UdpRecvTransport::listen(url)
            .map(|t| Box::new(BoundedUdpRecv { inner: t }) as Box<dyn RecvTransport>)
            .map_err(|e| format!("udp listen {url}: {e}")),
        "tcp" | "tcps" => tcp_transport(url).map(|t| Box::new(t) as Box<dyn RecvTransport>),
        "rist" => tst_rist::RistRecvTransport::listen(url)
            .map(|t| Box::new(t) as Box<dyn RecvTransport>)
            .map_err(|e| format!("rist listen {url}: {e}")),
        "srt" => srt_socket(url)
            .map(|s| Box::new(tst_srt::SrtTransport::new(s)) as Box<dyn RecvTransport>),
        other => Err(format!("unsupported scheme for recv: {other}://")),
    }
}

/// Scheme prefix of a `scheme://...` URL (e.g. `"udp"`, `"tcps"`).
fn scheme_of(url: &str) -> Result<&str, String> {
    url.split_once("://")
        .map(|(scheme, _)| scheme)
        .ok_or_else(|| format!("not a URL (missing '://'): {url}"))
}

/// Build a connected/accepted `TcpTransport` for either mode described
/// by the URL's `?listen=1` param (default caller). Shared by
/// [`make_send`] and [`make_recv`] — `TcpTransport` implements both
/// `Transport` and `RecvTransport` regardless of how it was built, and
/// `?listen=1` (which peer binds vs. connects) is an axis independent of
/// which direction (send/recv) this crate is using the resulting
/// transport for.
fn tcp_transport(url: &str) -> Result<tst_tcp::TcpTransport, String> {
    let parsed = tst_tcp::url::TcpUrl::parse(url).map_err(|e| format!("tcp url {url}: {e}"))?;
    if parsed.listen {
        tst_tcp::TcpListener::from_url(url)
            .map_err(|e| format!("tcp listen {url}: {e}"))?
            .accept_blocking()
            .map_err(|e| format!("tcp accept {url}: {e}"))
    } else {
        tst_tcp::TcpTransport::connect(url).map_err(|e| format!("tcp connect {url}: {e}"))
    }
}

/// Build a connected/accepted SRT `Socket` for either mode described by
/// the URL's `?mode=` param (default caller). Shared by [`make_send`]
/// and [`make_recv`] — `SrtTransport` implements both `Transport` and
/// `RecvTransport` over the same `Socket`, and SRT's caller/listener
/// axis is independent of which direction this crate is using the
/// resulting transport for. Follows the URL-overlay pattern from
/// `examples/sending/sender_from_url.rs`: build via a builder (for the
/// crate-side defaults above), clone out the config, apply the URL's
/// overlay (which wins on conflict), then connect/bind with the result.
fn srt_socket(url: &str) -> Result<tst_srt::Socket, String> {
    let parsed = tst_srt::SrtUrl::parse(url).map_err(|e| format!("srt url {url}: {e}"))?;
    match parsed.mode {
        tst_srt::url::Mode::Caller => {
            let mut b = tst_srt::SocketBuilder::new();
            b.recv_timeout(DEFAULT_SRT_RECV_TIMEOUT);
            b.connect_timeout(DEFAULT_SRT_CONNECT_TIMEOUT);
            let mut cfg = b.config();
            parsed.overlay.apply_to_socket(&mut cfg);
            tst_srt::Socket::connect_with(&cfg, (parsed.host.as_str(), parsed.port))
                .map_err(|e| format!("srt connect {url}: {e}"))
        }
        tst_srt::url::Mode::Listener => {
            let mut b = tst_srt::ListenerBuilder::new();
            b.recv_timeout(DEFAULT_SRT_RECV_TIMEOUT);
            let mut cfg = b.config();
            parsed.overlay.apply_to_listener(&mut cfg);
            // Empty host (`srt://:port?mode=listener`) means "bind every
            // interface" per `Mode::Listener`'s own doc — `SrtUrl` passes
            // the raw (possibly empty) host through unresolved, so the
            // wildcard substitution happens here, at the one call site
            // that actually binds a socket.
            let bind_host = if parsed.host.is_empty() {
                "0.0.0.0"
            } else {
                parsed.host.as_str()
            };
            let bind_addr = format!("{bind_host}:{}", parsed.port);
            let mut listener = tst_srt::Listener::bind_with(&cfg, bind_addr.as_str())
                .map_err(|e| format!("srt bind {url}: {e}"))?;
            let (socket, _peer) = listener
                .accept()
                .map_err(|e| format!("srt accept {url}: {e}"))?;
            Ok(socket)
        }
    }
}

/// UDP receiver adapter — see the module doc's "Bounded receive"
/// section. Drives `UdpRecvTransport::recv_timeout` (the type's own
/// documented escape hatch for cooperative shutdown) instead of the
/// trait-level `recv_bytes`, translating an idle poll into
/// `TransportError::Backpressure` so a caller-driven deadline loop
/// regains control periodically. Checks `is_alive()` up front so a
/// same-thread `close()` (see `RecvTransport::close`'s docs on the
/// same-thread-close-then-recv pattern) takes effect on the very next
/// call, rather than only after data would otherwise arrive.
struct BoundedUdpRecv {
    inner: tst_udp::UdpRecvTransport,
}

impl RecvTransport for BoundedUdpRecv {
    fn recv_bytes(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
        if !self.inner.is_alive() {
            return Err(TransportError::Closed);
        }
        match self.inner.recv_timeout(buf, UDP_RECV_POLL) {
            Ok(Some(n)) => Ok(n),
            Ok(None) => Err(TransportError::Backpressure {
                msg: "udp recv poll timeout".into(),
                errno_code: None,
            }),
            Err(e) => Err(TransportError::Broken {
                msg: e.to_string(),
                errno_code: None,
            }),
        }
    }

    fn max_payload(&self) -> usize {
        self.inner.max_payload()
    }

    fn is_alive(&self) -> bool {
        self.inner.is_alive()
    }

    fn close(&mut self) {
        self.inner.close();
    }

    fn socket_stats(&self) -> Option<SocketStats> {
        self.inner.socket_stats()
    }
}

/// Shared running tally a [`Teeing`] wrapper accumulates.
pub(crate) struct TeeState {
    bytes: u64,
    hasher: Sha256,
}

impl TeeState {
    fn new() -> Self {
        Self {
            bytes: 0,
            hasher: Sha256::new(),
        }
    }
}

/// Transport/RecvTransport wrapper that tees every byte it forwards
/// into a shared, running byte-count + sha256 hash — `send.rs`/
/// `recv.rs`'s ground truth for `CellMetrics::bytes`/`stream_sha256`,
/// computed at the transport boundary rather than derived from whatever
/// the muxer/demuxer parsed. `Arc<Mutex<..>>` rather than
/// `Rc<RefCell<..>>` because `Transport`/`RecvTransport` require `Send`
/// (the pipeline shells that own the wrapped transport must be movable
/// across threads in the general case), even though every caller in
/// this crate reads the tap back on the same thread that wrote it.
///
/// [`Self::new`] returns the wrapper plus a clone of its tap; hand the
/// wrapper to a pipeline shell (`MuxSender`/`DemuxReceiver`), then once
/// that shell has been dropped (releasing its clone), pass the tap to
/// [`tee_tally`] to read the final `(bytes, sha256_hex)`.
pub(crate) struct Teeing<T> {
    inner: T,
    tap: Arc<Mutex<TeeState>>,
}

impl<T> Teeing<T> {
    pub(crate) fn new(inner: T) -> (Self, Arc<Mutex<TeeState>>) {
        let tap = Arc::new(Mutex::new(TeeState::new()));
        (
            Self {
                inner,
                tap: Arc::clone(&tap),
            },
            tap,
        )
    }
}

impl<T: Transport> Transport for Teeing<T> {
    fn send_bytes(&mut self, msg: &[u8]) -> Result<(), TransportError> {
        {
            let mut s = self.tap.lock().expect("tee mutex poisoned");
            s.bytes += msg.len() as u64;
            s.hasher.update(msg);
        }
        self.inner.send_bytes(msg)
    }
    fn max_payload(&self) -> usize {
        self.inner.max_payload()
    }
    fn is_alive(&self) -> bool {
        self.inner.is_alive()
    }
    fn close(&mut self) {
        self.inner.close();
    }
    fn cancel_handle(&self) -> Option<Arc<dyn TransportCancel + Send + Sync>> {
        self.inner.cancel_handle()
    }
    fn socket_stats(&self) -> Option<SocketStats> {
        self.inner.socket_stats()
    }
}

impl<T: RecvTransport> RecvTransport for Teeing<T> {
    fn recv_bytes(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
        let n = self.inner.recv_bytes(buf)?;
        let mut s = self.tap.lock().expect("tee mutex poisoned");
        s.bytes += n as u64;
        s.hasher.update(&buf[..n]);
        Ok(n)
    }
    fn max_payload(&self) -> usize {
        self.inner.max_payload()
    }
    fn is_alive(&self) -> bool {
        self.inner.is_alive()
    }
    fn close(&mut self) {
        self.inner.close();
    }
    fn cancel_handle(&self) -> Option<Arc<dyn TransportCancel + Send + Sync>> {
        self.inner.cancel_handle()
    }
    fn socket_stats(&self) -> Option<SocketStats> {
        self.inner.socket_stats()
    }
}

/// Read back the final `(bytes, sha256_hex)` from a tap handle returned
/// by [`Teeing::new`]. Call only after the pipeline shell that owned
/// the `Teeing` has been dropped (releasing its clone of the tap) —
/// panics otherwise, since a live writer means the tally isn't final
/// yet.
pub(crate) fn tee_tally(tap: Arc<Mutex<TeeState>>) -> (u64, String) {
    let state = Arc::try_unwrap(tap)
        .unwrap_or_else(|_| panic!("tee_tally: tap still has another owner (shell not dropped?)"))
        .into_inner()
        .expect("tee mutex poisoned");
    (state.bytes, crate::verify::to_hex(&state.hasher.finalize()))
}
