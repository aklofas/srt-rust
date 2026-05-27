//! Socket knobs that apply to recv-side sockets (currently just SO_RCVBUF).

use std::net::UdpSocket;

use crate::config::SocketConfig;

pub fn apply_recv_knobs(socket: &UdpSocket, cfg: &SocketConfig) -> std::io::Result<()> {
    let s = socket2::SockRef::from(socket);
    if let Some(rcv) = cfg.rcvbuf {
        s.set_recv_buffer_size(rcv)?;
    }
    Ok(())
}
