//! Loopback unicast round-trip tests for UdpTransport + UdpRecvTransport.
//!
//! Sender test: plain std::net::UdpSocket on the receive side so we can
//! verify our sender independently. Receiver test: vice versa.

use std::net::UdpSocket;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use tst_core::transport::{RecvTransport, Transport};
use tst_udp::{UdpRecvTransport, UdpTransport};

#[test]
fn unicast_loopback_sends_payload_to_std_recv() {
    let recv = UdpSocket::bind("127.0.0.1:0").expect("bind recv");
    recv.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let port = recv.local_addr().unwrap().port();

    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    let _t = thread::spawn(move || {
        let mut buf = vec![0u8; 2048];
        if let Ok((n, _)) = recv.recv_from(&mut buf) {
            let _ = tx.send(buf[..n].to_vec());
        }
    });
    thread::sleep(Duration::from_millis(50));

    let url = format!("udp://127.0.0.1:{port}");
    let mut send = UdpTransport::connect(&url).expect("build sender");
    let payload = [0x47u8; 188];
    send.send_bytes(&payload).expect("send_bytes");

    let got = rx
        .recv_timeout(Duration::from_secs(2))
        .expect("recv timed out");
    assert_eq!(got.as_slice(), &payload[..]);
}

#[test]
fn unicast_loopback_sends_via_hostname_url() {
    // The containerized-consumer path: `udp://<name>:<port>` where <name>
    // is a resolvable hostname, not an IP literal. Bind the receiver on
    // whatever address the system resolver returns FIRST for localhost —
    // the same selection UdpUrl::parse uses — so the test holds whether
    // the resolver prefers ::1 or 127.0.0.1.
    use std::net::ToSocketAddrs;
    let first = ("localhost", 0u16)
        .to_socket_addrs()
        .expect("resolve localhost")
        .next()
        .expect("localhost resolved to no addresses");
    let recv = UdpSocket::bind(first).expect("bind recv");
    recv.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let port = recv.local_addr().unwrap().port();

    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    let _t = thread::spawn(move || {
        let mut buf = vec![0u8; 2048];
        if let Ok((n, _)) = recv.recv_from(&mut buf) {
            let _ = tx.send(buf[..n].to_vec());
        }
    });
    thread::sleep(Duration::from_millis(50));

    let url = format!("udp://localhost:{port}");
    let mut send = UdpTransport::connect(&url).expect("connect via hostname URL");
    let payload = [0x47u8; 188];
    send.send_bytes(&payload).expect("send_bytes");

    let got = rx
        .recv_timeout(Duration::from_secs(2))
        .expect("recv timed out");
    assert_eq!(got.as_slice(), &payload[..]);
}

#[test]
fn unicast_loopback_recvs_payload_from_std_send() {
    let mut recv = UdpRecvTransport::listen("udp://@127.0.0.1:0").expect("build recv");
    let local_port = recv.local_addr().port();

    let _t = thread::spawn(move || {
        thread::sleep(Duration::from_millis(50));
        let s = UdpSocket::bind("127.0.0.1:0").unwrap();
        s.connect(("127.0.0.1", local_port)).unwrap();
        s.send(&[0x47u8; 188]).unwrap();
    });

    let mut buf = vec![0u8; recv.max_payload()];
    let n = recv.recv_bytes(&mut buf).expect("recv_bytes");
    assert_eq!(n, 188);
    assert!(buf[..n].iter().all(|&b| b == 0x47));
}
