//! Socket knobs that apply to recv-side sockets (currently just SO_RCVBUF).

use std::net::UdpSocket;

use tst_core::net::udp_socket::set_socket_buffers;

use crate::config::SocketConfig;

pub fn apply_recv_knobs(socket: &UdpSocket, cfg: &SocketConfig) -> std::io::Result<()> {
    set_socket_buffers(socket, cfg.rcvbuf, None)
}
