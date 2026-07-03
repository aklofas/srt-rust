//! Socket knobs shared between TcpTransport's send and recv paths.

use std::io;
use std::net::TcpStream;

use crate::config::SocketConfig;
// Transport-agnostic cancel-poll cadence; it lives in the udp_socket module
// for historical reasons but is deliberately shared across transports.
use tst_core::net::udp_socket::{CANCEL_POLL_INTERVAL, set_socket_buffers};

pub fn apply_knobs(socket: &TcpStream, cfg: &SocketConfig) -> io::Result<()> {
    socket.set_read_timeout(Some(CANCEL_POLL_INTERVAL))?;
    socket.set_write_timeout(Some(CANCEL_POLL_INTERVAL))?;

    if let Some(nd) = cfg.nodelay {
        socket.set_nodelay(nd)?;
    }

    set_socket_buffers(socket, cfg.rcvbuf, cfg.sndbuf)?;
    if let Some(idle) = cfg.keepalive {
        let ka = socket2::TcpKeepalive::new().with_time(idle);
        socket2::SockRef::from(socket).set_tcp_keepalive(&ka)?;
    }
    Ok(())
}
