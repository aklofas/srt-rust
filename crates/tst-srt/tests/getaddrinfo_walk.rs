//! Verifies connect/bind walk past failing resolved addresses.
//! Audit Issues 3 + 10.

use tst_srt::{ListenerBuilder, SocketBuilder};
use std::net::ToSocketAddrs;
use std::thread;
use std::time::Duration;

#[test]
fn connect_walks_to_v4_when_v6_first_and_unbindable() {
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

    let accept_thread = thread::spawn(move || {
        let mut l = listener;
        l.accept().expect("accept")
    });

    thread::sleep(Duration::from_millis(50));

    let socket = SocketBuilder::new()
        .connect_timeout(Duration::from_secs(2))
        .send_timeout(Duration::from_secs(5))
        .connect(format!("localhost:{port}"))
        .expect("connect should walk past unbindable ::1 to bindable 127.0.0.1");

    let _ = accept_thread.join().expect("join");
    drop(socket);
}
