//! `MuxSender` over an in-memory `Transport` must produce TS bytes
//! byte-identical to the CI-guarded video-roundtrip golden. This is the std
//! mirror of the on-device check in `embedded/baremetal-qemu/`.

use std::sync::{Arc, Mutex};

use tst_core::mpegts::common::Pts90khz;
use tst_core::mpegts::mux::{MuxerConfig, MuxerProgramConfigBuilder, VideoCodec};
use tst_core::transport::{Transport, TransportError};
use tst_pipeline::MuxSender;

#[derive(Clone)]
struct Sink(Arc<Mutex<Vec<u8>>>);
impl Transport for Sink {
    fn send_bytes(&mut self, b: &[u8]) -> Result<(), TransportError> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(())
    }
    fn max_payload(&self) -> usize {
        1316
    }
    fn close(&mut self) {}
    fn is_alive(&self) -> bool {
        true
    }
}

/// Verbatim of `tst_integration::scenarios::synthetic_h264_idr()`.
fn synthetic_h264_idr() -> Vec<u8> {
    let mut buf = Vec::with_capacity(20);
    buf.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
    buf.push(0x65);
    for i in 0u8..15 {
        buf.push(0xA5 ^ i);
    }
    buf
}

#[test]
fn mux_sender_matches_video_roundtrip_golden() {
    let golden =
        include_bytes!("../../tst-integration/tests/fixtures/scenarios/video-roundtrip/output.ts");

    let cfg = {
        let mut prog = MuxerProgramConfigBuilder::new(1, 0x1000);
        prog.add_video(0x1011, VideoCodec::H264);
        let mut b = MuxerConfig::builder();
        b.add_program(prog.build());
        b.build().expect("valid muxer config")
    };

    let collected = Arc::new(Mutex::new(Vec::new()));
    let sender = MuxSender::new(Sink(Arc::clone(&collected)), cfg).expect("mux sender");
    sender
        .send_video(&synthetic_h264_idr(), Pts90khz::new(0), true)
        .expect("send_video");
    sender.close();

    assert_eq!(
        collected.lock().unwrap().as_slice(),
        golden.as_slice(),
        "MuxSender output diverged from the video-roundtrip golden"
    );
}
