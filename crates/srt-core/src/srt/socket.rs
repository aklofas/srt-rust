//! Socket type. Filled in Task 9.

use crate::error::ConnectError;
use crate::srt::config::SocketConfig;
use std::net::ToSocketAddrs;

pub struct Socket;
pub struct Stats;

impl Socket {
    pub fn connect_with(_config: &SocketConfig, _addr: impl ToSocketAddrs) -> Result<Socket, ConnectError> {
        unimplemented!("Task 9")
    }
}
