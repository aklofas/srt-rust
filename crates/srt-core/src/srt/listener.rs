//! Listener type. Filled in Task 10.

use crate::error::BindError;
use crate::srt::config::ListenerConfig;
use std::net::ToSocketAddrs;

pub struct Listener;

impl Listener {
    pub fn bind_with(_config: &ListenerConfig, _addr: impl ToSocketAddrs) -> Result<Listener, BindError> {
        unimplemented!("Task 10")
    }
}
