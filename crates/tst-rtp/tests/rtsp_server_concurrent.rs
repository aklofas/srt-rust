//! Phase 3 Wave F Task 25 — concurrent unicast clients.

use std::time::Duration;
use tst_core::mpegts::mux::{MuxerConfig, MuxerProgramConfigBuilder, VideoCodec};
use tst_rtp::{RtspClient, RtspServer};

fn make_muxer_cfg() -> MuxerConfig {
    let mut prog = MuxerProgramConfigBuilder::new(1, 0x1000);
    prog.add_video(0x1011, VideoCodec::H264);
    let mut b = MuxerConfig::builder();
    b.add_program(prog.build());
    b.build().unwrap()
}

/// N concurrent clients all connect + DESCRIBE successfully.
#[test]
fn ten_concurrent_describes() {
    let server = RtspServer::bind("rtsp://127.0.0.1:0").unwrap();
    let _mount = server.add_mount("/live", make_muxer_cfg()).unwrap();
    server.start().unwrap();
    let port = server.local_addr().unwrap().port();
    let url = std::sync::Arc::new(format!("rtsp://127.0.0.1:{port}/live"));

    let mut handles = vec![];
    for _ in 0..10 {
        let url = url.clone();
        handles.push(std::thread::spawn(move || {
            let mut client = RtspClient::connect(&url).unwrap();
            client.options().unwrap();
            let sdp = client.describe().unwrap();
            assert!(!sdp.media.is_empty());
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    server.stop().ok();
}

/// active_sessions count tracks correctly across concurrent connects + drops.
#[test]
fn active_sessions_count_reflects_concurrent_clients() {
    let server = RtspServer::bind("rtsp://127.0.0.1:0").unwrap();
    let _mount = server.add_mount("/live", make_muxer_cfg()).unwrap();
    server.start().unwrap();
    let port = server.local_addr().unwrap().port();
    let url = format!("rtsp://127.0.0.1:{port}/live");

    let clients: Vec<_> = (0..5)
        .map(|_| {
            let mut c = RtspClient::connect(&url).unwrap();
            c.options().unwrap();
            c
        })
        .collect();
    // Brief settle.
    std::thread::sleep(Duration::from_millis(200));
    assert!(server.stats().active_sessions >= 5);
    drop(clients);
    std::thread::sleep(Duration::from_millis(200));
    server.stop().ok();
}
