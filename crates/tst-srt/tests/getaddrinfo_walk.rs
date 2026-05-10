//! Verifies connect/bind walk past failing resolved addresses.
//! Audit Issues 3 + 10.

mod common;

use std::net::ToSocketAddrs;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;
use tst_srt::{ListenerBuilder, SocketBuilder};

#[test]
fn connect_walks_to_v4_when_v6_first_and_unbindable() {
    require_loopback!();
    // Bind a listener on 127.0.0.1 only. If we walk the addr iterator,
    // we should reach the v4 address even if ::1 comes first when
    // resolving "localhost".
    let listener = ListenerBuilder::new()
        .recv_timeout(Duration::from_secs(5))
        .bind("127.0.0.1:0")
        .expect("bind v4");
    let port = listener.local_addr().unwrap().port();

    // Sanity: confirm "localhost:<port>" resolves to multiple addresses.
    let addrs: Vec<_> = format!("localhost:{port}")
        .to_socket_addrs()
        .expect("resolve")
        .collect();
    if addrs.len() < 2 {
        eprintln!(
            "skipping: localhost resolved to only {} address(es)",
            addrs.len()
        );
        return;
    }

    let ready = Arc::new(AtomicBool::new(false));
    let r = ready.clone();
    let accept_thread = thread::spawn(move || {
        let mut l = listener;
        r.store(true, Ordering::SeqCst);
        l.accept().expect("accept")
    });

    common::wait_for_ready(&ready);

    let socket = SocketBuilder::new()
        .connect_timeout(Duration::from_secs(2))
        .send_timeout(Duration::from_secs(5))
        .connect(format!("localhost:{port}"))
        .expect("connect should walk past unbindable ::1 to bindable 127.0.0.1");

    let _ = accept_thread.join().expect("join");
    drop(socket);
}
