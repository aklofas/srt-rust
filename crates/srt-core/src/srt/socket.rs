//! Connected SRT data socket.

use crate::error::{
    ConnectError, IoError, OptionError, RecvError, SendError, SrtErrno, last_error,
};
use crate::init::ensure_initialized;
use crate::srt::addr::{from_sockaddr, to_sockaddr};
use crate::srt::config::SocketConfig;
use crate::srt::options::{MaxBandwidth, Passphrase};
use std::ffi::{c_char, c_int};
use std::mem;
use std::net::{SocketAddr, ToSocketAddrs};
use std::time::Duration;

const SRT_INVALID_SOCK: srt_sys::SRTSOCKET = -1;

/// Connected, bidirectional SRT data socket.
///
/// Constructed via [`SocketBuilder`](crate::srt::SocketBuilder), via
/// [`Socket::connect_with`], or returned from [`Listener::accept`](crate::srt::Listener::accept).
pub struct Socket {
    handle: srt_sys::SRTSOCKET,
    /// Shared close-once primitive. Cloned out via `cancel_handle()` so a
    /// thread parked in `send`/`recv` can be woken from another thread.
    /// Drop calls `cancel.cancel()` so explicit `close()` and Drop never
    /// double-close.
    cancel: crate::srt::CancelHandle,
    cached_stream_id: Option<String>,
    cached_payload_limit: usize,
}

/// Snapshot of libsrt's per-socket performance counters (subset of `CBytePerfMon`).
///
/// Loss/drop counters are split by which side observed the event:
/// - `*_send_side` — what the sender knows is lost/dropped (receiver NAKs received,
///   too-late drops on outgoing path). Read these on a sender; they will be
///   ~0 on a receiver.
/// - `*_recv_side` — what the receiver detected (sequence-gap discoveries,
///   too-late drops on incoming path). Read these on a receiver; they will
///   be ~0 on a sender.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct Stats {
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub bytes_lost_recv_side: u64,
    pub bytes_lost_send_side: u64,
    pub packets_sent: u64,
    pub packets_received: u64,
    pub packets_lost_recv_side: u64,
    pub packets_lost_send_side: u64,
    pub packets_retransmitted: u64,
    pub packets_dropped_recv_side: u64,
    pub packets_dropped_send_side: u64,
    pub rtt: Duration,
    pub send_bandwidth_bps: u64,
    pub recv_bandwidth_bps: u64,
    pub mbps_estimated_bandwidth: f64,
    pub send_buffer_packets: u32,
    pub recv_buffer_packets: u32,
}

// SAFETY: a SRTSOCKET is just an integer handle; libsrt is internally
// thread-safe per-socket (concurrent operations on the same handle from two
// threads are still UB, but moving across threads is fine). Mirrors std::net.
unsafe impl Send for Socket {}

impl Socket {
    /// Raw libsrt socket handle. **Unstable, used for low-level interop tests.**
    #[doc(hidden)]
    pub fn raw_handle(&self) -> srt_sys::SRTSOCKET {
        self.handle
    }

    /// Open a socket, apply config, and connect to `addr`.
    ///
    /// `addr` may resolve to multiple `SocketAddr`s (e.g. `localhost` →
    /// `[::1]:N` + `127.0.0.1:N`). We walk every resolved address in the
    /// order returned by `to_socket_addrs()` and return the first
    /// successful connection. On dual-stack hosts where AAAA records
    /// resolve before A but v6 is unroutable (tethered cellular, some
    /// VPNs, some corporate networks), this lets the v4 fallback succeed
    /// instead of failing fast on `[::1]`. Mirrors ffmpeg's
    /// `getaddrinfo(AF_UNSPEC)` + `ai_next` walk in
    /// `libavformat/libsrt.c`. Sequential, no Happy Eyeballs.
    pub fn connect_with(
        config: &SocketConfig,
        addr: impl ToSocketAddrs,
    ) -> Result<Self, ConnectError> {
        ensure_initialized();

        let addrs: Vec<SocketAddr> = addr
            .to_socket_addrs()
            .map_err(|e| ConnectError::InvalidAddress(e.into()))?
            .collect();
        if addrs.is_empty() {
            return Err(ConnectError::InvalidAddress(
                crate::error::AddrError::Resolve("no addresses resolved".into()),
            ));
        }

        let mut last_err: Option<ConnectError> = None;
        for sa in addrs {
            // Each iteration starts on a fresh handle. libsrt PRE options
            // must be set before srt_connect; once a handle has been
            // through a failed srt_connect we can't reuse it cleanly.
            let handle = unsafe { srt_sys::srt_create_socket() };
            if handle == SRT_INVALID_SOCK {
                last_err = Some(last_error().into());
                continue;
            }

            if let Err(e) = apply_socket_config(handle, config) {
                unsafe { srt_sys::srt_close(handle) };
                last_err = Some(ConnectError::InvalidOption(e));
                continue;
            }

            let (raw_sa, salen) = match to_sockaddr(sa) {
                Ok(p) => p,
                Err(e) => {
                    unsafe { srt_sys::srt_close(handle) };
                    last_err = Some(ConnectError::InvalidAddress(e));
                    continue;
                }
            };
            let rc =
                unsafe { srt_sys::srt_connect(handle, (&raw const raw_sa).cast(), salen as c_int) };
            if rc < 0 {
                let raw = last_error();
                unsafe { srt_sys::srt_close(handle) };
                last_err = Some(classify_connect_error(raw));
                continue;
            }

            let cached_stream_id = read_stream_id(handle);
            let cached_payload_limit = read_payload_size(handle);

            return Ok(Self {
                handle,
                cancel: make_cancel_handle(handle),
                cached_stream_id,
                cached_payload_limit,
            });
        }
        Err(last_err.expect("non-empty addrs always populates last_err on full-walk failure"))
    }

    /// Internal: wrap an already-accepted handle (called from `Listener::accept`).
    #[allow(dead_code)]
    pub(crate) fn from_accepted(
        handle: srt_sys::SRTSOCKET,
        send_timeout: Option<Duration>,
        recv_timeout: Option<Duration>,
    ) -> Result<Self, IoError> {
        if let Some(t) = send_timeout {
            set_int(
                handle,
                srt_sys::SRT_SOCKOPT_SRTO_SNDTIMEO,
                duration_to_ms(t),
            )
            .map_err(io_from_option_error)?;
        }
        if let Some(t) = recv_timeout {
            set_int(
                handle,
                srt_sys::SRT_SOCKOPT_SRTO_RCVTIMEO,
                duration_to_ms(t),
            )
            .map_err(io_from_option_error)?;
        }
        let cached_stream_id = read_stream_id(handle);
        let cached_payload_limit = read_payload_size(handle);
        Ok(Self {
            handle,
            cancel: make_cancel_handle(handle),
            cached_stream_id,
            cached_payload_limit,
        })
    }

    /// Send a buffer. Returns bytes sent. Live mode requires `buf.len() ≤ payload_size`.
    pub fn send(&mut self, buf: &[u8]) -> Result<usize, SendError> {
        let n = unsafe {
            srt_sys::srt_send(
                self.handle,
                buf.as_ptr().cast::<c_char>(),
                buf.len() as c_int,
            )
        };
        if n >= 0 {
            return Ok(n as usize);
        }
        let raw = last_error();
        Err(classify_send_error(
            raw,
            buf.len(),
            self.cached_payload_limit,
        ))
    }

    /// Receive into a buffer. Returns bytes received (one libsrt message).
    pub fn recv(&mut self, buf: &mut [u8]) -> Result<usize, RecvError> {
        let n = unsafe {
            srt_sys::srt_recv(
                self.handle,
                buf.as_mut_ptr().cast::<c_char>(),
                buf.len() as c_int,
            )
        };
        if n >= 0 {
            return Ok(n as usize);
        }
        let raw = last_error();
        Err(classify_recv_error(raw, buf.len()))
    }

    pub fn peer_addr(&self) -> Result<SocketAddr, IoError> {
        let mut storage: libc::sockaddr_storage = unsafe { mem::zeroed() };
        let mut len = mem::size_of::<libc::sockaddr_storage>() as c_int;
        let rc = unsafe {
            srt_sys::srt_getpeername(self.handle, (&raw mut storage).cast(), &raw mut len)
        };
        if rc < 0 {
            return Err(last_error().into());
        }
        from_sockaddr(&storage).map_err(|e| IoError::System(std::io::Error::other(e.to_string())))
    }

    pub fn local_addr(&self) -> Result<SocketAddr, IoError> {
        let mut storage: libc::sockaddr_storage = unsafe { mem::zeroed() };
        let mut len = mem::size_of::<libc::sockaddr_storage>() as c_int;
        let rc = unsafe {
            srt_sys::srt_getsockname(self.handle, (&raw mut storage).cast(), &raw mut len)
        };
        if rc < 0 {
            return Err(last_error().into());
        }
        from_sockaddr(&storage).map_err(|e| IoError::System(std::io::Error::other(e.to_string())))
    }

    /// Stream ID negotiated during handshake. Cached at construction.
    pub fn stream_id(&self) -> Option<&str> {
        self.cached_stream_id.as_deref()
    }

    /// Snapshot of libsrt's per-socket performance counters.
    pub fn stats(&self) -> Result<Stats, IoError> {
        let mut perf: srt_sys::CBytePerfMon = unsafe { mem::zeroed() };
        let rc = unsafe { srt_sys::srt_bistats(self.handle, &raw mut perf, 0, 0) };
        if rc < 0 {
            return Err(last_error().into());
        }
        Ok(perf_to_stats(&perf))
    }

    pub fn set_send_timeout(&mut self, timeout: Option<Duration>) -> Result<(), OptionError> {
        let ms = timeout.map(duration_to_ms).unwrap_or(-1);
        set_int(self.handle, srt_sys::SRT_SOCKOPT_SRTO_SNDTIMEO, ms)
    }

    pub fn set_recv_timeout(&mut self, timeout: Option<Duration>) -> Result<(), OptionError> {
        let ms = timeout.map(duration_to_ms).unwrap_or(-1);
        set_int(self.handle, srt_sys::SRT_SOCKOPT_SRTO_RCVTIMEO, ms)
    }

    pub fn set_max_bandwidth(&mut self, bw: MaxBandwidth) -> Result<(), OptionError> {
        set_i64(
            self.handle,
            srt_sys::SRT_SOCKOPT_SRTO_MAXBW,
            bw.as_libsrt_i64(),
        )
    }

    pub fn set_input_bandwidth(&mut self, bw: u64) -> Result<(), OptionError> {
        set_i64(self.handle, srt_sys::SRT_SOCKOPT_SRTO_INPUTBW, bw as i64)
    }

    pub fn set_overhead_bandwidth_pct(&mut self, pct: u8) -> Result<(), OptionError> {
        if !(5..=100).contains(&pct) {
            return Err(OptionError::OutOfRange(format!(
                "overhead_bandwidth_pct must be 5..=100, got {pct}"
            )));
        }
        set_int(self.handle, srt_sys::SRT_SOCKOPT_SRTO_OHEADBW, pct as i32)
    }

    /// Explicit close; rare. Drop handles the normal path.
    ///
    /// After `close()` returns, any other thread parked in `send`/`recv`
    /// on a clone of this socket's `cancel_handle()` observes
    /// `SendError::ConnectionBroken` / `RecvError::ConnectionBroken`.
    pub fn close(self) -> Result<(), IoError> {
        // CancelHandle::cancel does the srt_close and is idempotent. We
        // can't easily plumb the rc back out (closer is `Fn`), so the
        // Result type stays for back-compat but always returns Ok.
        self.cancel.cancel();
        Ok(())
    }

    /// Clone-able close handle. Calling `cancel()` from any thread
    /// closes the underlying SRT socket — wakes a peer thread parked in
    /// `send` or `recv` with a Broken-class error. Idempotent.
    pub fn cancel_handle(&self) -> crate::srt::CancelHandle {
        self.cancel.clone()
    }
}

impl Drop for Socket {
    fn drop(&mut self) {
        // No-op if explicit close() / cancel() already fired.
        self.cancel.cancel();
    }
}

// ============================================================================
// Helpers (private to crate; consumed by listener.rs in Task 10)
// ============================================================================

pub(crate) fn duration_to_ms(d: Duration) -> i32 {
    d.as_millis().min(i32::MAX as u128) as i32
}

pub(crate) fn set_int(
    handle: srt_sys::SRTSOCKET,
    opt: srt_sys::SRT_SOCKOPT,
    value: i32,
) -> Result<(), OptionError> {
    let rc = unsafe {
        srt_sys::srt_setsockflag(
            handle,
            opt,
            (&raw const value).cast(),
            mem::size_of::<i32>() as c_int,
        )
    };
    if rc < 0 {
        return Err(last_error().into());
    }
    Ok(())
}

pub(crate) fn set_i64(
    handle: srt_sys::SRTSOCKET,
    opt: srt_sys::SRT_SOCKOPT,
    value: i64,
) -> Result<(), OptionError> {
    let rc = unsafe {
        srt_sys::srt_setsockflag(
            handle,
            opt,
            (&raw const value).cast(),
            mem::size_of::<i64>() as c_int,
        )
    };
    if rc < 0 {
        return Err(last_error().into());
    }
    Ok(())
}

pub(crate) fn set_bool(
    handle: srt_sys::SRTSOCKET,
    opt: srt_sys::SRT_SOCKOPT,
    value: bool,
) -> Result<(), OptionError> {
    let v: i32 = if value { 1 } else { 0 };
    set_int(handle, opt, v)
}

pub(crate) fn set_string(
    handle: srt_sys::SRTSOCKET,
    opt: srt_sys::SRT_SOCKOPT,
    value: &str,
) -> Result<(), OptionError> {
    let rc = unsafe {
        srt_sys::srt_setsockflag(handle, opt, value.as_ptr().cast(), value.len() as c_int)
    };
    if rc < 0 {
        return Err(last_error().into());
    }
    Ok(())
}

/// Set `SRTO_LINGER`. Unlike most SRT options it takes a `struct linger`
/// (not an `int`), so we can't go through `set_int`. `Duration::ZERO` (or
/// any sub-second duration) disables linger entirely (`l_onoff = 0`),
/// causing `srt_close` to return immediately and discard any unsent
/// payload. Non-zero seconds are clamped into `i32` range.
pub(crate) fn set_linger(handle: srt_sys::SRTSOCKET, d: Duration) -> Result<(), OptionError> {
    let secs = d.as_secs().min(i32::MAX as u64) as c_int;
    let lin = libc::linger {
        l_onoff: if secs > 0 { 1 } else { 0 },
        l_linger: secs,
    };
    let rc = unsafe {
        srt_sys::srt_setsockopt(
            handle,
            0,
            srt_sys::SRT_SOCKOPT_SRTO_LINGER,
            (&raw const lin).cast(),
            std::mem::size_of::<libc::linger>() as c_int,
        )
    };
    if rc < 0 {
        return Err(last_error().into());
    }
    Ok(())
}

pub(crate) fn set_passphrase(
    handle: srt_sys::SRTSOCKET,
    p: &Passphrase,
) -> Result<(), OptionError> {
    let bytes = p.as_bytes();
    let rc = unsafe {
        srt_sys::srt_setsockflag(
            handle,
            srt_sys::SRT_SOCKOPT_SRTO_PASSPHRASE,
            bytes.as_ptr().cast(),
            bytes.len() as c_int,
        )
    };
    if rc < 0 {
        return Err(last_error().into());
    }
    Ok(())
}

/// Apply every set field of a `SocketConfig` to the handle.
pub(crate) fn apply_socket_config(
    handle: srt_sys::SRTSOCKET,
    cfg: &SocketConfig,
) -> Result<(), OptionError> {
    if let Some(p) = &cfg.passphrase {
        // Set key length BEFORE passphrase.
        set_int(
            handle,
            srt_sys::SRT_SOCKOPT_SRTO_PBKEYLEN,
            cfg.key_length.as_bytes(),
        )?;
        set_passphrase(handle, p)?;
    }
    if let Some(t) = cfg.send_timeout {
        set_int(
            handle,
            srt_sys::SRT_SOCKOPT_SRTO_SNDTIMEO,
            duration_to_ms(t),
        )?;
    }
    if let Some(t) = cfg.recv_timeout {
        set_int(
            handle,
            srt_sys::SRT_SOCKOPT_SRTO_RCVTIMEO,
            duration_to_ms(t),
        )?;
    }
    if let Some(t) = cfg.connect_timeout {
        // SRTO_CONNTIMEO is a PRE option (set before srt_connect); this
        // function is called between srt_create_socket and srt_connect, so
        // ordering is satisfied.
        set_int(
            handle,
            srt_sys::SRT_SOCKOPT_SRTO_CONNTIMEO,
            duration_to_ms(t),
        )?;
    }
    if let Some(d) = cfg.linger {
        set_linger(handle, d)?;
    }
    if let Some(d) = cfg.latency {
        set_int(handle, srt_sys::SRT_SOCKOPT_SRTO_LATENCY, duration_to_ms(d))?;
    }
    if let Some(d) = cfg.peer_latency {
        set_int(
            handle,
            srt_sys::SRT_SOCKOPT_SRTO_PEERLATENCY,
            duration_to_ms(d),
        )?;
    }
    if let Some(d) = cfg.recv_latency {
        set_int(
            handle,
            srt_sys::SRT_SOCKOPT_SRTO_RCVLATENCY,
            duration_to_ms(d),
        )?;
    }
    if let Some(n) = cfg.recv_buf_packets {
        set_int(handle, srt_sys::SRT_SOCKOPT_SRTO_RCVBUF, n as i32)?;
    }
    if let Some(n) = cfg.send_buf_packets {
        set_int(handle, srt_sys::SRT_SOCKOPT_SRTO_SNDBUF, n as i32)?;
    }
    if let Some(bw) = cfg.max_bandwidth {
        set_i64(handle, srt_sys::SRT_SOCKOPT_SRTO_MAXBW, bw.as_libsrt_i64())?;
    }
    if let Some(bw) = cfg.input_bandwidth {
        set_i64(handle, srt_sys::SRT_SOCKOPT_SRTO_INPUTBW, bw as i64)?;
    }
    if let Some(pct) = cfg.overhead_bandwidth_pct {
        if !(5..=100).contains(&pct) {
            return Err(OptionError::OutOfRange(format!(
                "overhead_bandwidth_pct must be 5..=100, got {pct}"
            )));
        }
        set_int(handle, srt_sys::SRT_SOCKOPT_SRTO_OHEADBW, pct as i32)?;
    }
    if let Some(mss) = cfg.mss {
        set_int(handle, srt_sys::SRT_SOCKOPT_SRTO_MSS, mss as i32)?;
    }
    if let Some(n) = cfg.payload_size {
        set_int(handle, srt_sys::SRT_SOCKOPT_SRTO_PAYLOADSIZE, n as i32)?;
    }
    if let Some(n) = cfg.udp_recv_buffer_bytes {
        set_int(handle, srt_sys::SRT_SOCKOPT_SRTO_UDP_RCVBUF, n as i32)?;
    }
    if let Some(n) = cfg.udp_send_buffer_bytes {
        set_int(handle, srt_sys::SRT_SOCKOPT_SRTO_UDP_SNDBUF, n as i32)?;
    }
    if let Some(id) = &cfg.stream_id {
        set_string(handle, srt_sys::SRT_SOCKOPT_SRTO_STREAMID, id.as_str())?;
    }
    if let Some(n) = cfg.loss_max_ttl {
        set_int(handle, srt_sys::SRT_SOCKOPT_SRTO_LOSSMAXTTL, n as i32)?;
    }
    if let Some(on) = cfg.too_late_packet_drop {
        set_bool(handle, srt_sys::SRT_SOCKOPT_SRTO_TLPKTDROP, on)?;
    }
    if let Some(n) = cfg.flow_window_packets {
        set_int(handle, srt_sys::SRT_SOCKOPT_SRTO_FC, n as i32)?;
    }
    if let Some(pf) = &cfg.packet_filter {
        set_string(handle, srt_sys::SRT_SOCKOPT_SRTO_PACKETFILTER, pf.as_str())?;
    }
    if let Some(c) = cfg.congestion {
        set_string(handle, srt_sys::SRT_SOCKOPT_SRTO_CONGESTION, c.as_str())?;
    }
    if matches!(cfg.role, crate::srt::options::Role::Sender) {
        set_bool(handle, srt_sys::SRT_SOCKOPT_SRTO_SENDER, true)?;
    }
    Ok(())
}

pub(crate) fn apply_listener_config(
    handle: srt_sys::SRTSOCKET,
    cfg: &crate::srt::config::ListenerConfig,
) -> Result<(), OptionError> {
    if let Some(p) = &cfg.passphrase {
        set_int(
            handle,
            srt_sys::SRT_SOCKOPT_SRTO_PBKEYLEN,
            cfg.key_length.as_bytes(),
        )?;
        set_passphrase(handle, p)?;
    }
    if let Some(d) = cfg.latency {
        set_int(handle, srt_sys::SRT_SOCKOPT_SRTO_LATENCY, duration_to_ms(d))?;
    }
    if let Some(d) = cfg.recv_latency {
        set_int(
            handle,
            srt_sys::SRT_SOCKOPT_SRTO_RCVLATENCY,
            duration_to_ms(d),
        )?;
    }
    if let Some(n) = cfg.recv_buf_packets {
        set_int(handle, srt_sys::SRT_SOCKOPT_SRTO_RCVBUF, n as i32)?;
    }
    if let Some(bw) = cfg.max_bandwidth {
        set_i64(handle, srt_sys::SRT_SOCKOPT_SRTO_MAXBW, bw.as_libsrt_i64())?;
    }
    if let Some(pct) = cfg.overhead_bandwidth_pct {
        if !(5..=100).contains(&pct) {
            return Err(OptionError::OutOfRange(format!(
                "overhead_bandwidth_pct must be 5..=100, got {pct}"
            )));
        }
        set_int(handle, srt_sys::SRT_SOCKOPT_SRTO_OHEADBW, pct as i32)?;
    }
    if let Some(mss) = cfg.mss {
        set_int(handle, srt_sys::SRT_SOCKOPT_SRTO_MSS, mss as i32)?;
    }
    if let Some(n) = cfg.payload_size {
        set_int(handle, srt_sys::SRT_SOCKOPT_SRTO_PAYLOADSIZE, n as i32)?;
    }
    if let Some(n) = cfg.udp_recv_buffer_bytes {
        set_int(handle, srt_sys::SRT_SOCKOPT_SRTO_UDP_RCVBUF, n as i32)?;
    }
    if let Some(n) = cfg.loss_max_ttl {
        set_int(handle, srt_sys::SRT_SOCKOPT_SRTO_LOSSMAXTTL, n as i32)?;
    }
    if let Some(on) = cfg.too_late_packet_drop {
        set_bool(handle, srt_sys::SRT_SOCKOPT_SRTO_TLPKTDROP, on)?;
    }
    if let Some(n) = cfg.flow_window_packets {
        set_int(handle, srt_sys::SRT_SOCKOPT_SRTO_FC, n as i32)?;
    }
    if let Some(pf) = &cfg.packet_filter {
        set_string(handle, srt_sys::SRT_SOCKOPT_SRTO_PACKETFILTER, pf.as_str())?;
    }
    if let Some(c) = cfg.congestion {
        set_string(handle, srt_sys::SRT_SOCKOPT_SRTO_CONGESTION, c.as_str())?;
    }
    set_bool(handle, srt_sys::SRT_SOCKOPT_SRTO_REUSEADDR, cfg.reuse_addr)?;
    if let Some(t) = cfg.recv_timeout {
        set_int(
            handle,
            srt_sys::SRT_SOCKOPT_SRTO_RCVTIMEO,
            duration_to_ms(t),
        )?;
    }
    if let Some(d) = cfg.linger {
        set_linger(handle, d)?;
    }
    Ok(())
}

pub(crate) fn read_stream_id(handle: srt_sys::SRTSOCKET) -> Option<String> {
    let mut buf = [0u8; 513];
    let mut len = buf.len() as c_int;
    let rc = unsafe {
        srt_sys::srt_getsockflag(
            handle,
            srt_sys::SRT_SOCKOPT_SRTO_STREAMID,
            buf.as_mut_ptr().cast(),
            &raw mut len,
        )
    };
    if rc < 0 || len <= 0 {
        return None;
    }
    let s = std::str::from_utf8(&buf[..len as usize]).ok()?;
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

pub(crate) fn read_payload_size(handle: srt_sys::SRTSOCKET) -> usize {
    let mut value: c_int = 0;
    let mut len = std::mem::size_of::<c_int>() as c_int;
    let rc = unsafe {
        srt_sys::srt_getsockflag(
            handle,
            srt_sys::SRT_SOCKOPT_SRTO_PAYLOADSIZE,
            (&raw mut value).cast(),
            &raw mut len,
        )
    };
    if rc < 0 || value <= 0 {
        // SRT_LIVE_DEF_PLSIZE = 1316 (8 x 188-byte TS packets).
        return 1316;
    }
    value as usize
}

pub(crate) fn perf_to_stats(p: &srt_sys::CBytePerfMon) -> Stats {
    Stats {
        bytes_sent: p.byteSentTotal,
        bytes_received: p.byteRecvTotal,
        bytes_lost_recv_side: p.byteRcvLossTotal,
        // CBytePerfMon doesn't expose byteSndLossTotal — sender-side loss is
        // only reported as packet count (pktSndLossTotal). We surface 0 for
        // bytes-lost-send-side; consumers should use packets_lost_send_side.
        bytes_lost_send_side: 0,
        packets_sent: p.pktSentTotal as u64,
        packets_received: p.pktRecvTotal as u64,
        packets_lost_recv_side: p.pktRcvLossTotal as u64,
        packets_lost_send_side: p.pktSndLossTotal as u64,
        packets_retransmitted: p.pktRetransTotal as u64,
        packets_dropped_recv_side: p.pktRcvDropTotal as u64,
        packets_dropped_send_side: p.pktSndDropTotal as u64,
        rtt: Duration::from_millis(p.msRTT.max(0.0) as u64),
        send_bandwidth_bps: (p.mbpsSendRate * 1_000_000.0).max(0.0) as u64,
        recv_bandwidth_bps: (p.mbpsRecvRate * 1_000_000.0).max(0.0) as u64,
        mbps_estimated_bandwidth: p.mbpsBandwidth,
        send_buffer_packets: p.pktSndBuf as u32,
        recv_buffer_packets: p.pktRcvBuf as u32,
    }
}

#[allow(dead_code)]
fn io_from_option_error(e: OptionError) -> IoError {
    match e {
        OptionError::Other { kind, message } => IoError::Other { kind, message },
        other => IoError::Other {
            kind: SrtErrno::Unknown(0),
            message: other.to_string(),
        },
    }
}

fn classify_connect_error(raw: crate::error::RawError) -> ConnectError {
    raw.into()
}

fn classify_send_error(raw: crate::error::RawError, payload_len: usize, limit: usize) -> SendError {
    // Deterministic check: if the caller's buffer obviously exceeds the
    // configured payload size, classify regardless of libsrt's specific
    // error wording (which has shifted across versions).
    if payload_len > limit {
        return SendError::PayloadTooLarge {
            actual: payload_len,
            limit,
        };
    }
    if raw.message.contains("Message has no destination address")
        || raw.message.contains("payload size")
        || raw.message.contains("Invalid argument")
        || raw
            .message
            .contains("Incorrect use of Message API (sendmsg/recvmsg)")
    {
        return SendError::PayloadTooLarge {
            actual: payload_len,
            limit,
        };
    }
    raw.into()
}

fn classify_recv_error(raw: crate::error::RawError, buf_len: usize) -> RecvError {
    if raw.message.contains("Message size exceeds buffer") {
        return RecvError::BufferTooSmall {
            buf_len,
            message_len: 0,
        };
    }
    raw.into()
}

/// Build a CancelHandle that closes the SRTSOCKET on first cancel.
fn make_cancel_handle(handle: srt_sys::SRTSOCKET) -> crate::srt::CancelHandle {
    crate::srt::CancelHandle::new(handle as i64, |h| {
        // SAFETY: h was the same SRTSOCKET we stored; libsrt accepts
        // srt_close from any thread; the atomic-swap in CancelHandle
        // guarantees this runs at most once.
        let _ = unsafe { srt_sys::srt_close(h as srt_sys::SRTSOCKET) };
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn stats_struct_has_role_split_loss_fields() {
        // Compile-check that the new public fields exist.
        let _ = |s: super::Stats| {
            let _ = s.packets_lost_recv_side;
            let _ = s.packets_lost_send_side;
            let _ = s.bytes_lost_recv_side;
            let _ = s.bytes_lost_send_side;
            let _ = s.packets_dropped_recv_side;
            let _ = s.packets_dropped_send_side;
        };
    }

    /// Construction-shape test that doesn't need a live socket: building
    /// a Socket from a fake handle and double-closing it should NOT
    /// double-call srt_close. Without an integration test running real
    /// libsrt, we verify by checking that explicit close().is_ok() and
    /// drop() coexist safely (drop() must skip srt_close when cancel
    /// already ran).
    #[test]
    fn double_close_via_cancel_then_drop_is_safe() {
        // We construct a CancelHandle by hand around a fake handle with a
        // closer that records the call count, mirroring what Socket holds.
        use crate::srt::CancelHandle;
        use std::sync::atomic::{AtomicU32, Ordering};
        let calls = std::sync::Arc::new(AtomicU32::new(0));
        let calls_cl = calls.clone();
        let cancel = CancelHandle::new(99, move |_| {
            calls_cl.fetch_add(1, Ordering::SeqCst);
        });
        // Cancel once (simulates Socket::close).
        cancel.cancel();
        // Drop the handle (simulates Socket::drop calling cancel.cancel()
        // a second time).
        cancel.cancel();
        drop(cancel);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
