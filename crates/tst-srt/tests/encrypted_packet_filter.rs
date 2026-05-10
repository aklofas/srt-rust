//! Reed-Solomon FEC config applies. Doesn't assert recovery quality.

mod common;

use std::thread;
use std::time::Duration;
use tst_srt::{ListenerBuilder, PacketFilter, SocketBuilder};

#[test]
fn fec_config_applies() {
    require_loopback!();
    let pf_listener = PacketFilter::new("fec,cols:10,rows:5,arq:onreq").unwrap();
    let pf_caller = PacketFilter::new("fec,cols:10,rows:5,arq:onreq").unwrap();

    let mut listener = ListenerBuilder::new()
        .packet_filter(pf_listener)
        .recv_timeout(Duration::from_secs(5))
        .bind("127.0.0.1:0")
        .expect("bind");
    let port = listener.local_addr().unwrap().port();

    let lh = thread::spawn(move || {
        let _ = listener.accept().expect("accept");
    });
    common::settle();

    let _socket = SocketBuilder::new()
        .packet_filter(pf_caller)
        .recv_timeout(Duration::from_secs(5))
        .connect(format!("127.0.0.1:{port}"))
        .expect("connect");

    lh.join().unwrap();
}
