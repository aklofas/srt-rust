//! End-to-end integration test: pipeline::MuxSender over a real Socket pair.

mod common;

use std::thread;
use std::time::Duration;
use tst_core::mpegts::mux::MuxerConfig;
use tst_pipeline::MuxSender;
use tst_srt::SocketBuilder;
use tst_srt::SrtTransport;
use tst_test_helpers::synthetic_nal;

#[test]
fn sender_round_trip_one_frame() {
    require_loopback!();
    let lb = common::Loopback::bind();
    let port = lb.port;

    // Recv thread: receive bytes until peer closes; return total received.
    let accept = lb.spawn_accept(|mut peer| {
        let mut total = 0usize;
        let mut buf = vec![0u8; 1316];
        while let Ok(n) = peer.recv(&mut buf) {
            if n == 0 {
                break;
            }
            total += n;
        }
        total
    });
    accept.wait_ready();

    // MuxSender thread: connect + send one frame.
    let socket = SocketBuilder::new()
        .latency(Duration::from_millis(120))
        .connect(format!("127.0.0.1:{port}"))
        .expect("connect");
    let transport = SrtTransport::new(socket);
    let sender = MuxSender::new(transport, MuxerConfig::default()).expect("sender");

    let nal = synthetic_nal::h264_au(500, true);
    let klv = synthetic_nal::klv_blob(64);
    sender.send_video(&nal, 0, true).expect("send_video");
    sender.send_klv(&klv, 0, 0x00).expect("send_klv");

    // Brief pause to let bytes drain on the wire before close.
    thread::sleep(Duration::from_millis(200));
    sender.close();

    let total_bytes = accept.join();
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
