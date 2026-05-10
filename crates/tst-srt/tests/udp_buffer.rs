//! Verifies SRTO_UDP_RCVBUF / SRTO_UDP_SNDBUF setters reach libsrt.
//! Audit Issue 9.

mod common;

use std::ffi::c_int;
use std::thread;
use std::time::Duration;
use tst_srt::{ListenerBuilder, SocketBuilder};

fn read_u32(handle: srt_sys::SRTSOCKET, opt: srt_sys::SRT_SOCKOPT) -> Option<u32> {
    let mut value: c_int = 0;
    let mut len = std::mem::size_of::<c_int>() as c_int;
    let rc =
        unsafe { srt_sys::srt_getsockflag(handle, opt, (&raw mut value).cast(), &raw mut len) };
    if rc < 0 || value < 0 {
        return None;
    }
    Some(value as u32)
}

#[test]
fn udp_buffer_sizes_round_trip() {
    require_loopback!();
    let listener = ListenerBuilder::new()
        .udp_recv_buffer_bytes(2_000_000)
        .recv_timeout(Duration::from_secs(5))
        .bind("127.0.0.1:0")
        .expect("bind");
    let port = listener.local_addr().unwrap().port();

    let accept_thread = thread::spawn(move || {
        let mut l = listener;
        l.accept().expect("accept")
    });

    thread::sleep(Duration::from_millis(50));

    let socket = SocketBuilder::new()
        .udp_recv_buffer_bytes(2_000_000)
        .udp_send_buffer_bytes(2_000_000)
        .send_timeout(Duration::from_secs(5))
        .connect(("127.0.0.1", port))
        .expect("connect");

    let _ = accept_thread.join().expect("join");

    let handle = socket.raw_handle();
    let rcv = read_u32(handle, srt_sys::SRT_SOCKOPT_SRTO_UDP_RCVBUF).expect("read RCVBUF");
    let snd = read_u32(handle, srt_sys::SRT_SOCKOPT_SRTO_UDP_SNDBUF).expect("read SNDBUF");

    // Kernel may clamp to net.core.{r,w}mem_max; verify it's well above
    // OS default (~208 KB on Linux). Use 1 MB as conservative floor.
    assert!(rcv >= 1_000_000, "UDP_RCVBUF too small: {rcv}");
    assert!(snd >= 1_000_000, "UDP_SNDBUF too small: {snd}");
}
