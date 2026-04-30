//! Passive (listening) SRT socket.

use crate::error::{last_error, AcceptError, BindError, IoError, OptionError};
use crate::init::ensure_initialized;
use crate::srt::addr::{from_sockaddr, to_sockaddr};
use crate::srt::config::ListenerConfig;
use crate::srt::socket::{
    apply_listener_config, duration_to_ms, set_int, Socket,
};
use std::ffi::c_int;
use std::mem;
use std::net::{SocketAddr, ToSocketAddrs};
use std::time::Duration;

const SRT_INVALID_SOCK: srt_sys::SRTSOCKET = -1;

/// Passive socket. Created by [`ListenerBuilder`](crate::srt::ListenerBuilder)
/// or [`Listener::bind_with`].
pub struct Listener {
    handle: srt_sys::SRTSOCKET,
    /// Stored for inheritance into accepted Sockets.
    accepted_send_timeout: Option<Duration>,
    accepted_recv_timeout: Option<Duration>,
}

unsafe impl Send for Listener {}

impl Listener {
    pub fn bind_with(
        config: &ListenerConfig,
        addr: impl ToSocketAddrs,
    ) -> Result<Self, BindError> {
        ensure_initialized();

        let addr = addr
            .to_socket_addrs()
            .map_err(|e| BindError::InvalidAddress(e.into()))?
            .next()
            .ok_or_else(|| {
                BindError::InvalidAddress(crate::error::AddrError::Resolve(
                    "no addresses resolved".into(),
                ))
            })?;

        let handle = unsafe { srt_sys::srt_create_socket() };
        if handle == SRT_INVALID_SOCK {
            return Err(last_error().into());
        }

        if let Err(e) = apply_listener_config(handle, config) {
            unsafe { srt_sys::srt_close(handle) };
            return Err(BindError::InvalidOption(e));
        }

        let (sa, salen) = to_sockaddr(addr).map_err(BindError::InvalidAddress)?;
        let rc = unsafe { srt_sys::srt_bind(handle, (&raw const sa).cast(), salen as c_int) };
        if rc < 0 {
            let raw = last_error();
            unsafe { srt_sys::srt_close(handle) };
            return Err(raw.into());
        }

        let rc = unsafe { srt_sys::srt_listen(handle, config.backlog as c_int) };
        if rc < 0 {
            let raw = last_error();
            unsafe { srt_sys::srt_close(handle) };
            return Err(raw.into());
        }

        Ok(Self {
            handle,
            accepted_send_timeout: config.send_timeout,
            accepted_recv_timeout: config.recv_timeout,
        })
    }

    /// Block until an incoming connection completes the SRT handshake.
    pub fn accept(&mut self) -> Result<(Socket, SocketAddr), AcceptError> {
        let mut storage: libc::sockaddr_storage = unsafe { mem::zeroed() };
        let mut len = mem::size_of::<libc::sockaddr_storage>() as c_int;

        let accepted = unsafe {
            srt_sys::srt_accept(self.handle, (&raw mut storage).cast(), &raw mut len)
        };
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

    pub fn set_recv_timeout(&mut self, timeout: Option<Duration>) -> Result<(), OptionError> {
        let ms = timeout.map(duration_to_ms).unwrap_or(-1);
        set_int(self.handle, srt_sys::SRT_SOCKOPT_SRTO_RCVTIMEO, ms)?;
        self.accepted_recv_timeout = timeout;
        Ok(())
    }

    pub fn close(self) -> Result<(), IoError> {
        let handle = self.handle;
        std::mem::forget(self);
        let rc = unsafe { srt_sys::srt_close(handle) };
        if rc < 0 {
            return Err(last_error().into());
        }
        Ok(())
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        unsafe { srt_sys::srt_close(self.handle) };
    }
}
