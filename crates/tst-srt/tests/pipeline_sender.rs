//! End-to-end integration test: pipeline::MuxSender over a real Socket pair.

mod common;

use common::synthetic_nal;
use std::thread;
use std::time::Duration;
use tst_core::mpegts::mux::Config;
use tst_pipeline::MuxSender;
use tst_srt::SrtTransport;
use tst_srt::{ListenerBuilder, SocketBuilder};

#[test]
fn sender_round_trip_one_frame() {
    // Listener side: bind to ephemeral port.
    let mut listener = ListenerBuilder::new()
        .bind("127.0.0.1:0")
        .expect("bind listener");
    let port = listener.local_addr().unwrap().port();

    // Caller thread: waits for incoming, recv loop, drops bytes.
    let recv_handle = thread::spawn(move || {
        let (mut peer, _addr) = listener.accept().expect("accept");
        let mut total = 0usize;
        let mut buf = vec![0u8; 1316];
        // Recv until the peer closes (we'll close after one frame).
        while let Ok(n) = peer.recv(&mut buf) {
            if n == 0 {
                break;
            }
            total += n;
        }
        total
    });

    // MuxSender thread: connect + send one frame.
    let socket = SocketBuilder::new()
        .latency(Duration::from_millis(120))
        .connect(format!("127.0.0.1:{port}"))
        .expect("connect");
    let transport = SrtTransport::new(socket);
    let sender = MuxSender::new(Config::default(), transport).expect("sender");

    let nal = synthetic_nal::h264_au(500, true);
    let klv = synthetic_nal::klv_blob(64);
    sender.send_video(&nal, 0, true).expect("send_video");
    sender.send_klv(&klv, 0, 0x00).expect("send_klv");

    // Brief pause to let bytes drain on the wire before close.
    thread::sleep(Duration::from_millis(200));
    sender.close();

    let total_bytes = recv_handle.join().expect("recv thread");
    assert!(
        total_bytes > 0,
        "receiver should have got bytes; got {total_bytes}"
    );
    // Basic sanity: output must be TS-aligned (188-byte packets).
    assert_eq!(
        total_bytes % 188,
        0,
        "expected TS-aligned output, got {total_bytes} bytes"
    );
}
