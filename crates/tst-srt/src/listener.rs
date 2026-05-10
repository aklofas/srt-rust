//! Passive (listening) SRT socket.

use crate::addr::{from_sockaddr, to_sockaddr};
use crate::config::ListenerConfig;
use crate::error::{AcceptError, BindError, IoError, OptionError, last_error};
use crate::init::ensure_initialized;
use crate::socket::{Socket, apply_listener_config, duration_to_ms, set_int};
use std::ffi::c_int;
use std::mem;
use std::net::{SocketAddr, ToSocketAddrs};
use std::time::Duration;

const SRT_INVALID_SOCK: srt_sys::SRTSOCKET = -1;

// SRT_EPOLL_IN = 1 per srt.h. Signals a listener fd has an incoming
// connection ready for srt_accept. The generated constant is typed as
// SRT_EPOLL_OPT (c_uint) but srt_epoll_add_usock expects *const c_int.
const SRT_EPOLL_IN: c_int = 0x1;

/// Passive socket. Created by [`ListenerBuilder`](crate::ListenerBuilder)
/// or [`Listener::bind_with`].
pub struct Listener {
    handle: srt_sys::SRTSOCKET,
    /// Shared close-once primitive. Cloned out via `cancel_handle()` so a
    /// thread parked in `accept` can be woken from another thread. Drop
    /// calls `cancel.cancel()` so explicit `close()` and Drop never
    /// double-close.
    cancel: tst_core::CancelHandle,
    /// Stored for inheritance into accepted Sockets.
    accepted_send_timeout: Option<Duration>,
    accepted_recv_timeout: Option<Duration>,
}

unsafe impl Send for Listener {}

impl Listener {
    /// Open a passive socket, apply config, bind, and start listening.
    ///
    /// Walks every address resolved from `addr` in iterator order; first
    /// successful bind+listen wins. Same rationale as
    /// [`Socket::connect_with`]: on dual-stack hosts the iterator may
    /// return AAAA before A and the v6 entry may be unbindable (e.g. v6
    /// disabled on the interface), so we fall through to v4. Mirrors
    /// ffmpeg's `ai_next` walk.
    ///
    /// # Panics
    ///
    /// On the very first libsrt-touching call in the process, this
    /// triggers `srt_startup()` and panics if libsrt fails to initialize
    /// (returns `< 0`). That is a process-fatal condition — libsrt cannot
    /// be used at all from this process — so a panic is the correct
    /// signal. Subsequent calls reuse the once-initialized state and do
    /// not re-trigger the startup path.
    pub fn bind_with(config: &ListenerConfig, addr: impl ToSocketAddrs) -> Result<Self, BindError> {
        ensure_initialized();

        let addrs: Vec<SocketAddr> = addr
            .to_socket_addrs()
            .map_err(|e| BindError::InvalidAddress(e.into()))?
            .collect();
        if addrs.is_empty() {
            return Err(BindError::InvalidAddress(crate::error::AddrError::Resolve(
                "no addresses resolved".into(),
            )));
        }

        let mut last_err: Option<BindError> = None;
        for sa in addrs {
            let handle = unsafe { srt_sys::srt_create_socket() };
            if handle == SRT_INVALID_SOCK {
                last_err = Some(last_error().into());
                continue;
            }

            if let Err(e) = apply_listener_config(handle, config) {
                unsafe { srt_sys::srt_close(handle) };
                last_err = Some(BindError::InvalidOption(e));
                continue;
            }

            let (raw_sa, salen) = match to_sockaddr(sa) {
                Ok(p) => p,
                Err(e) => {
                    unsafe { srt_sys::srt_close(handle) };
                    last_err = Some(BindError::InvalidAddress(e));
                    continue;
                }
            };
            let rc =
                unsafe { srt_sys::srt_bind(handle, (&raw const raw_sa).cast(), salen as c_int) };
            if rc < 0 {
                let raw = last_error();
                unsafe { srt_sys::srt_close(handle) };
                last_err = Some(raw.into());
                continue;
            }

            let rc = unsafe { srt_sys::srt_listen(handle, config.backlog as c_int) };
            if rc < 0 {
                let raw = last_error();
                unsafe { srt_sys::srt_close(handle) };
                last_err = Some(raw.into());
                continue;
            }

            return Ok(Self {
                handle,
                cancel: make_cancel_handle(handle),
                accepted_send_timeout: config.send_timeout,
                accepted_recv_timeout: config.recv_timeout,
            });
        }
        Err(last_err.expect("non-empty addrs always populates last_err on full-walk failure"))
    }

    /// Block until an incoming connection completes the SRT handshake.
    pub fn accept(&mut self) -> Result<(Socket, SocketAddr), AcceptError> {
        let mut storage: libc::sockaddr_storage = unsafe { mem::zeroed() };
        let mut len = mem::size_of::<libc::sockaddr_storage>() as c_int;

        let accepted =
            unsafe { srt_sys::srt_accept(self.handle, (&raw mut storage).cast(), &raw mut len) };
        if accepted == SRT_INVALID_SOCK {
            return Err(last_error().into());
        }

        let socket = Socket::from_accepted(
            accepted,
            self.accepted_send_timeout,
            self.accepted_recv_timeout,
        )
        .map_err(|e| match e {
            IoError::Other { kind, message } => AcceptError::Other { kind, message },
            IoError::SocketClosed => AcceptError::ListenerClosed,
            IoError::System(io) => AcceptError::System(io),
        })?;

        let peer = from_sockaddr(&storage)
            .map_err(|e| AcceptError::System(std::io::Error::other(e.to_string())))?;
        Ok((socket, peer))
    }

    /// Accept the next incoming connection, returning
    /// [`AcceptError::TimedOut`] if no peer connects within `timeout`.
    ///
    /// **Why this exists:** libsrt's `srt_accept` does not honor the
    /// `SRTO_RCVTIMEO` socket option — [`set_recv_timeout`](Self::set_recv_timeout)
    /// on the listener inherits to accepted sockets but does *not* gate
    /// the accept call itself. This method works around that via
    /// `srt_epoll_wait`.
    ///
    /// Implementation: registers the listener fd with a one-shot epoll
    /// set on `SRT_EPOLL_IN`, waits up to `timeout`, then either calls
    /// `srt_accept` (connection ready) or returns `TimedOut`. The epoll
    /// set is always released before this method returns.
    pub fn accept_timeout(
        &mut self,
        timeout: Duration,
    ) -> Result<(Socket, SocketAddr), AcceptError> {
        // Create an epoll set for this call only.
        let eid = unsafe { srt_sys::srt_epoll_create() };
        if eid < 0 {
            return Err(AcceptError::Other {
                kind: crate::error::SrtErrno::Unknown(eid),
                message: format!("srt_epoll_create returned {eid}"),
            });
        }

        // Register the listener fd for IN (= accept-ready) events.
        // Bind to a local binding so &raw const has a place expression.
        let events: c_int = SRT_EPOLL_IN;
        let rc = unsafe { srt_sys::srt_epoll_add_usock(eid, self.handle, &raw const events) };
        if rc < 0 {
            let raw = last_error();
            unsafe { srt_sys::srt_epoll_release(eid) };
            return Err(raw.into());
        }

        // msTimeOut is i64; clamp Duration to the representable range.
        let timeout_ms: i64 = timeout.as_millis().min(i64::MAX as u128) as i64;
        let mut readfds: [srt_sys::SRTSOCKET; 1] = [SRT_INVALID_SOCK];
        let mut rnum: c_int = 1;

        let n = unsafe {
            srt_sys::srt_epoll_wait(
                eid,
                readfds.as_mut_ptr(),
                &raw mut rnum,
                std::ptr::null_mut(), // writefds
                std::ptr::null_mut(), // wnum
                timeout_ms,
                std::ptr::null_mut(), // lrfds (system sockets)
                std::ptr::null_mut(), // lrnum
                std::ptr::null_mut(), // lwfds
                std::ptr::null_mut(), // lwnum
            )
        };

        unsafe { srt_sys::srt_epoll_release(eid) };

        if n == 0 {
            // epoll_wait returns 0 when msTimeOut elapses with no event.
            return Err(AcceptError::TimedOut);
        }
        if n < 0 {
            return Err(last_error().into());
        }

        // n > 0: listener fd is accept-ready; delegate to the blocking path.
        // srt_accept will return immediately because epoll confirmed readiness.
        self.accept()
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

    /// Set the receive timeout that will be inherited by accepted [`Socket`]s.
    ///
    /// **Important:** this does *not* gate the [`accept`](Self::accept) call
    /// itself — libsrt's `srt_accept` does not honor `SRTO_RCVTIMEO`. To
    /// time-bound the accept call, use [`accept_timeout`](Self::accept_timeout)
    /// instead.
    pub fn set_recv_timeout(&mut self, timeout: Option<Duration>) -> Result<(), OptionError> {
        let ms = timeout.map(duration_to_ms).unwrap_or(-1);
        set_int(self.handle, srt_sys::SRT_SOCKOPT_SRTO_RCVTIMEO, ms)?;
        self.accepted_recv_timeout = timeout;
        Ok(())
    }

    /// Explicit close. Wakes any thread parked in `accept()` with
    /// `AcceptError::ListenerClosed`. Drop also calls cancel; both paths
    /// are idempotent.
    ///
    /// **Always returns `Ok`.** The `Result` is retained for API stability
    /// and may carry an error in a future revision (the underlying
    /// `srt_close` rc is currently swallowed by the `CancelHandle` closer).
    pub fn close(self) -> Result<(), IoError> {
        self.cancel.cancel();
        Ok(())
    }

    /// Clone-able close handle. Calling `cancel()` from any thread
    /// closes the underlying SRT listener socket. Idempotent.
    pub fn cancel_handle(&self) -> tst_core::CancelHandle {
        self.cancel.clone()
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        // No-op if explicit close() / cancel() already fired.
        self.cancel.cancel();
    }
}

/// Build a CancelHandle that closes the SRTSOCKET on first cancel.
fn make_cancel_handle(handle: srt_sys::SRTSOCKET) -> tst_core::CancelHandle {
    tst_core::CancelHandle::new(handle as i64, |h| {
        // SAFETY: h was the same SRTSOCKET we stored; libsrt accepts
        // srt_close from any thread; the atomic-swap in CancelHandle
        // guarantees this runs at most once.
        let _ = unsafe { srt_sys::srt_close(h as srt_sys::SRTSOCKET) };
    })
}
