//! Reed-Solomon FEC config applies. Doesn't assert recovery quality.

mod common;

use std::time::Duration;
use tst_srt::{ListenerBuilder, PacketFilter, SocketBuilder};

#[test]
fn fec_config_applies() {
    require_loopback!();
    let pf_listener = PacketFilter::new("fec,cols:10,rows:5,arq:onreq").unwrap();
    let pf_caller = PacketFilter::new("fec,cols:10,rows:5,arq:onreq").unwrap();

    let mut builder = ListenerBuilder::new();
    builder
        .packet_filter(pf_listener)
        .recv_timeout(Duration::from_secs(5));
    let lb = common::Loopback::bind_with(builder);
    let port = lb.port;

    let accept = lb.spawn_accept(|sock| {
        drop(sock);
    });
    accept.wait_ready();

    let _socket = SocketBuilder::new()
        .packet_filter(pf_caller)
        .recv_timeout(Duration::from_secs(5))
        .connect(format!("127.0.0.1:{port}"))
        .expect("connect");

    accept.join();
}
