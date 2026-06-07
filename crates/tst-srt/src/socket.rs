//! Connected SRT data socket.

use crate::addr::{from_sockaddr, to_sockaddr};
use crate::config::SocketConfig;
use crate::error::{
    ConnectError, IoError, OptionError, RecvError, SendError, SrtErrno, classify_connect_error,
    last_error, last_reject,
};
use crate::init::ensure_initialized;
use crate::options::{MaxBandwidth, Passphrase};
use os_socketaddr::OsSocketAddr;
use std::ffi::{c_char, c_int};
use std::mem;
use std::net::{SocketAddr, ToSocketAddrs};
use std::time::Duration;
use tst_core::mpegts::common::SRT_TS_BUNDLE_BYTES;

const SRT_INVALID_SOCK: srt_sys::SRTSOCKET = -1;

/// Connected, bidirectional SRT data socket.
///
/// Constructed via [`SocketBuilder`](crate::SocketBuilder), via
/// [`Socket::connect_with`], or returned from [`Listener::accept`](crate::Listener::accept).
///
/// # Closing
///
/// `Socket` is `Send` (libsrt is internally per-handle thread-safe; the
/// `SRTSOCKET` integer can move across threads). It supports three
/// shutdown patterns:
///
/// 1. **Drop** — the [`Drop`] impl calls `cancel.cancel()`, which fires
///    `srt_close(fd)` exactly once (idempotent with `close()`). Bounded
///    by `SRTO_LINGER` (libsrt default 30 s, configurable via
///    `SocketBuilder::linger` before construction).
/// 2. **Explicit close** — call [`Self::close`] (consuming `self`).
///    Equivalent to drop's cancel; always returns `Ok(())` (the inner
///    `srt_close` rc is currently swallowed; see method doc).
/// 3. **Cross-thread cancel** — call [`Self::cancel_handle`] to obtain a
///    [`tst_core::SrtCancelHandle`] (clone-able, `Send + Sync`), then
///    `cancel()` from any thread. Closes the libsrt socket; a peer
///    parked in `send` / `recv` returns
///    [`SendError::ConnectionBroken`] / [`RecvError::ConnectionBroken`]
///    within one libsrt I/O cycle (~3-10 ms).
///
/// ## Per-language idiom
///
/// | Language | Idiom |
/// |----------|-------|
/// | Rust | `let _ = socket;` (Drop) or `socket.cancel_handle().cancel();` (cross-thread) |
/// | Java | Wrap as `AutoCloseable`; `try-with-resources` calls drop on exit |
/// | Kotlin | Wrap as `AutoCloseable`; `.use { }` calls drop on exit |
/// | Swift | `deinit` calls drop; `defer { handle.cancel() }` for explicit cross-thread |
/// | Python | Wrap as `__enter__`/`__exit__`; `with ... as sock:` calls drop on exit |
/// | C | (deferred — `Socket` is not directly exposed at the C ABI; senders/receivers wrap it) |
///
/// See [`docs/reference/srt-cancel-handle.md`](https://github.com/aklofas/ts-transformer/blob/main/ts-transformer/docs/reference/srt-cancel-handle.md) for the full cancel-handle pattern.
pub struct Socket {
    handle: srt_sys::SRTSOCKET,
    /// Shared close-once primitive. Cloned out via `cancel_handle()` so a
    /// thread parked in `send`/`recv` can be woken from another thread.
    /// Drop calls `cancel.cancel()` so explicit `close()` and Drop never
    /// double-close.
    cancel: tst_core::SrtCancelHandle,
    /// Cached at construction; libsrt allows reading via getsockflag, but
    /// reading once is cheaper.
    cached_stream_id: Option<String>,
    /// `SRTO_PAYLOADSIZE` read after handshake. Used to give accurate
    /// `SendError::PayloadTooLarge { limit }` values without a per-send
    /// getsockopt round-trip. Defaults to [`SRT_TS_BUNDLE_BYTES`] (libsrt live default) if read fails.
    cached_payload_limit: usize,
}

impl std::fmt::Debug for Socket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Socket")
            .field("handle", &self.handle)
            .field("stream_id", &self.cached_stream_id)
            .field("payload_limit", &self.cached_payload_limit)
            .finish()
    }
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
#[must_use]
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
    ///
    /// # Panics
    ///
    /// On the very first libsrt-touching call in the process, this
    /// triggers `srt_startup()` and panics if libsrt fails to initialize
    /// (returns `< 0`). That is a process-fatal condition — libsrt cannot
    /// be used at all from this process — so a panic is the correct
    /// signal. Subsequent calls reuse the once-initialized state and do
    /// not re-trigger the startup path.
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

            let os_addr = to_sockaddr(sa);
            let rc = unsafe {
                srt_sys::srt_connect(handle, os_addr.as_ptr().cast(), os_addr.len() as c_int)
            };
            if rc < 0 {
                let raw = last_error();
                // MUST read the reject reason from the live handle BEFORE
                // srt_close: once closed, libsrt's locateSocket returns null
                // and srt_getrejectreason always yields SRT_REJ_UNKNOWN.
                let reason = last_reject(handle);
                unsafe { srt_sys::srt_close(handle) };
                last_err = Some(classify_connect_error(raw, reason));
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
        let mut os_addr = OsSocketAddr::new();
        let mut len = os_addr.capacity() as c_int;
        let rc = unsafe {
            srt_sys::srt_getpeername(self.handle, os_addr.as_mut_ptr().cast(), &raw mut len)
        };
        if rc < 0 {
            return Err(last_error().into());
        }
        from_sockaddr(&os_addr).map_err(|e| IoError::System(std::io::Error::other(e.to_string())))
    }

    pub fn local_addr(&self) -> Result<SocketAddr, IoError> {
        let mut os_addr = OsSocketAddr::new();
        let mut len = os_addr.capacity() as c_int;
        let rc = unsafe {
            srt_sys::srt_getsockname(self.handle, os_addr.as_mut_ptr().cast(), &raw mut len)
        };
        if rc < 0 {
            return Err(last_error().into());
        }
        from_sockaddr(&os_addr).map_err(|e| IoError::System(std::io::Error::other(e.to_string())))
    }

    /// Stream ID negotiated during handshake. Cached at construction.
    pub fn stream_id(&self) -> Option<&str> {
        self.cached_stream_id.as_deref()
    }

    /// Post-handshake `SRTO_PAYLOADSIZE`, cached at construction.
    ///
    /// Returned in bytes. After the SRT handshake completes, both peers
    /// have agreed on a payload size (the smaller of the two configured
    /// values). This is the upper bound on the per-message length passed
    /// to [`Self::send`] in live mode — exceeding it causes
    /// [`SendError::PayloadTooLarge`].
    ///
    /// Defaults to libsrt's `SRT_LIVE_DEF_PLSIZE` (1316 bytes — i.e.
    /// [`tst_core::mpegts::common::SRT_TS_BUNDLE_BYTES`]) when libsrt's
    /// option-readback fails or returns a non-positive value.
    ///
    /// Used by [`SrtTransport::new`] to derive its `max_payload`; callers
    /// that build their own transport wrappers should query the same way.
    ///
    /// [`SendError::PayloadTooLarge`]: crate::error::SendError::PayloadTooLarge
    /// [`SrtTransport::new`]: crate::SrtTransport::new
    pub fn payload_limit(&self) -> usize {
        self.cached_payload_limit
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
    ///
    /// **Always returns `Ok`.** The `Result` is retained for API stability
    /// and may carry an error in a future revision (the underlying
    /// `srt_close` rc is currently swallowed by the `SrtCancelHandle` closer).
    pub fn close(self) -> Result<(), IoError> {
        // SrtCancelHandle::cancel does the srt_close and is idempotent. We
        // can't easily plumb the rc back out (closer is `Fn`), so the
        // Result type stays for back-compat but always returns Ok.
        self.cancel.cancel();
        Ok(())
    }

    /// Clone-able close handle. Calling `cancel()` from any thread
    /// closes the underlying SRT socket — wakes a peer thread parked in
    /// `send` or `recv` with a Broken-class error. Idempotent.
    pub fn cancel_handle(&self) -> tst_core::SrtCancelHandle {
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

pub(crate) fn read_bool(
    handle: srt_sys::SRTSOCKET,
    opt: srt_sys::SRT_SOCKOPT,
) -> Result<bool, OptionError> {
    let mut value: c_int = 0;
    let mut len = std::mem::size_of::<c_int>() as c_int;
    let rc =
        unsafe { srt_sys::srt_getsockflag(handle, opt, (&raw mut value).cast(), &raw mut len) };
    if rc < 0 {
        return Err(last_error().into());
    }
    Ok(value != 0)
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

/// `struct linger` for `SRTO_LINGER`, hand-rolled because the `libc` crate
/// doesn't expose `linger` on `*-pc-windows-msvc`.
///
/// The ABI is NOT the same across platforms, and libsrt uses each platform's
/// native definition: POSIX `struct linger` is two `int` (8 bytes), but
/// Win32/Winsock `LINGER` is two `u_short` (4 bytes). libsrt validates the
/// option length with `cast_optval<linger>` which throws `MJ_NOTSUP/MN_INVAL`
/// ("Operation not supported: Bad parameters") when `optlen != sizeof(linger)`
/// (socketconfig.h:368-371) — so a POSIX-sized 8-byte struct is rejected on
/// Windows, breaking every sender connect (sender_defaults sets `linger`).
/// Match the field widths per platform so `size_of::<LingerOpt>()` equals the
/// `sizeof(linger)` libsrt expects.
#[cfg(windows)]
#[repr(C)]
struct LingerOpt {
    l_onoff: core::ffi::c_ushort,
    l_linger: core::ffi::c_ushort,
}
#[cfg(not(windows))]
#[repr(C)]
struct LingerOpt {
    l_onoff: c_int,
    l_linger: c_int,
}

/// Set `SRTO_LINGER`. Unlike most SRT options it takes a `struct linger`
/// (not an `int`), so we can't go through `set_int`. `Duration::ZERO` (or
/// any sub-second duration) disables linger entirely (`l_onoff = 0`),
/// causing `srt_close` to return immediately and discard any unsent
/// payload. Non-zero seconds are clamped into `i32` range.
pub(crate) fn set_linger(handle: srt_sys::SRTSOCKET, d: Duration) -> Result<(), OptionError> {
    // Field types differ per platform (see `LingerOpt`): Win32 LINGER fields
    // are `u_short` (cap at u16::MAX seconds), POSIX `int`.
    #[cfg(windows)]
    let lin = {
        let secs = d.as_secs().min(u16::MAX as u64) as core::ffi::c_ushort;
        LingerOpt {
            l_onoff: if secs > 0 { 1 } else { 0 },
            l_linger: secs,
        }
    };
    #[cfg(not(windows))]
    let lin = {
        let secs = d.as_secs().min(i32::MAX as u64) as c_int;
        LingerOpt {
            l_onoff: if secs > 0 { 1 } else { 0 },
            l_linger: secs,
        }
    };
    let rc = unsafe {
        srt_sys::srt_setsockopt(
            handle,
            0,
            srt_sys::SRT_SOCKOPT_SRTO_LINGER,
            (&raw const lin).cast(),
            std::mem::size_of::<LingerOpt>() as c_int,
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
    if matches!(cfg.role, crate::options::Role::Sender) {
        set_bool(handle, srt_sys::SRT_SOCKOPT_SRTO_SENDER, true)?;
    }
    Ok(())
}

pub(crate) fn apply_listener_config(
    handle: srt_sys::SRTSOCKET,
    cfg: &crate::config::ListenerConfig,
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
        // libsrt's SRT_LIVE_DEF_PLSIZE — see SRT_TS_BUNDLE_BYTES.
        return SRT_TS_BUNDLE_BYTES;
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

fn io_from_option_error(e: OptionError) -> IoError {
    match e {
        OptionError::Other { kind, message } => IoError::Other { kind, message },
        other => IoError::Other {
            kind: SrtErrno::Unknown(0),
            message: other.to_string(),
        },
    }
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

/// Build a SrtCancelHandle that closes the SRTSOCKET on first cancel.
fn make_cancel_handle(handle: srt_sys::SRTSOCKET) -> tst_core::SrtCancelHandle {
    tst_core::SrtCancelHandle::new(handle as i64, |h| {
        // SAFETY: h was the same SRTSOCKET we stored; libsrt accepts
        // srt_close from any thread; the atomic-swap in SrtCancelHandle
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
        // We construct a SrtCancelHandle by hand around a fake handle with a
        // closer that records the call count, mirroring what Socket holds.
        use std::sync::atomic::{AtomicU32, Ordering};
        use tst_core::SrtCancelHandle;
        let calls = std::sync::Arc::new(AtomicU32::new(0));
        let calls_cl = calls.clone();
        let cancel = SrtCancelHandle::new(99, move |_| {
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
