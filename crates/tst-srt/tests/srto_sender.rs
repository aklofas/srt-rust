//! Verifies SRTO_SENDER=1 is set when role=MuxSender. Audit Issue 2.

use tst_srt::{ListenerBuilder, Role, SocketBuilder};
use std::ffi::c_int;
use std::thread;
use std::time::Duration;

fn read_srto_sender(handle: srt_sys::SRTSOCKET) -> i32 {
    // Initialize to 0 — libsrt's getsockflag for SRTO_SENDER writes a single
    // byte (the bool) and updates `len` to 1, leaving the upper 3 bytes
    // untouched. Pre-zeroing keeps the read deterministic.
    let mut value: c_int = 0;
    let mut len = std::mem::size_of::<c_int>() as c_int;
    let rc = unsafe {
        srt_sys::srt_getsockflag(
            handle,
            srt_sys::SRT_SOCKOPT_SRTO_SENDER,
            (&raw mut value).cast(),
            &raw mut len,
        )
    };
    assert!(rc >= 0, "srt_getsockflag(SRTO_SENDER) failed");
    value
}

#[test]
fn role_sender_sets_srto_sender_on_caller() {
    let listener = ListenerBuilder::new()
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
        .role(Role::MuxSender)
        .send_timeout(Duration::from_secs(5))
        .connect(("127.0.0.1", port))
        .expect("connect");

    let _ = accept_thread.join().expect("join");

    assert_eq!(
        read_srto_sender(socket.raw_handle()),
        1,
        "expected SRTO_SENDER=1 on caller with role=MuxSender",
    );
}

#[test]
fn role_unspecified_leaves_srto_sender_at_default() {
    let listener = ListenerBuilder::new()
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
        .send_timeout(Duration::from_secs(5))
        .connect(("127.0.0.1", port))
        .expect("connect");

    let _ = accept_thread.join().expect("join");

    assert_eq!(
        read_srto_sender(socket.raw_handle()),
        0,
        "expected SRTO_SENDER=0 (libsrt default) when role unspecified",
    );
}
