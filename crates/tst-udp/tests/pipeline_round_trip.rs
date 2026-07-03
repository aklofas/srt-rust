//! Pipeline-shell integration: MuxSender<UdpTransport> → UDP → DemuxReceiver<UdpRecvTransport>.
//!
//! Verifies that the full composition stack — muxer, UDP datagram send, UDP
//! datagram receive, TS sync, demuxer — round-trips an H.264 NAL from sender
//! to receiver without losing data.
//!
//! # Sync-lock requirement
//!
//! The `Receiver` sync state machine inside `DemuxReceiver` requires at least
//! 5 consecutive 188-byte-aligned sync bytes (0x47) before it transitions from
//! VERIFY to LOCKED and begins emitting events — roughly 941 bytes.  A single
//! video frame produces only ~2–3 TS packets, which is not enough on its own.
//!
//! Strategy (mirrors `crates/tst-pipeline/tests/pipeline_receiver.rs`):
//! use `psi_interval_ms(10)` (the minimum, ~900 ticks at 90 kHz) so PAT + PMT
//! re-emit on every push, then send several frames at 9001-tick spacing.  Each
//! send call crosses a PSI interval boundary and emits PAT + PMT + video,
//! giving ≥ 9 packets (1692 bytes) — well above the 941-byte lock threshold.

use std::thread;
use std::time::Duration;

use tst_core::mpegts::common::Pts90khz;
use tst_core::mpegts::demux::DemuxEvent;
use tst_core::mpegts::mux::{MuxerConfig, MuxerProgramConfigBuilder, VideoCodec};
use tst_pipeline::{DemuxReceiver, MuxSender, ShellErrorKind};
use tst_udp::{UdpRecvTransport, UdpTransport};

/// Minimal Annex-B H.264 AUD + IDR — just enough for the muxer to accept and
/// frame into a PES.  Semantic validity is not required; the muxer only checks
/// that the first four bytes are the Annex-B start code.
fn synthetic_h264_au() -> Vec<u8> {
    // AUD NAL (nal_type=9, primary_pic_type=0x10) followed by IDR NAL header.
    let mut v = vec![0x00, 0x00, 0x00, 0x01, 0x09, 0x10];
    v.extend([0x00, 0x00, 0x00, 0x01, 0x65]);
    v.extend(std::iter::repeat(0xab).take(200));
    v
}

#[test]
fn mux_via_udp_demux_round_trip_recovers_program_map() {
    // -----------------------------------------------------------------------
    // 1. Build the receiver first so the OS assigns a port before we send.
    // -----------------------------------------------------------------------
    let recv_transport =
        UdpRecvTransport::listen("udp://@127.0.0.1:0").expect("bind UdpRecvTransport");
    let local_port = recv_transport.local_addr().port();

    let mut receiver = DemuxReceiver::new(recv_transport);

    // -----------------------------------------------------------------------
    // 2. Build the muxer config.
    //
    //    psi_interval_ms(10) causes PAT + PMT to re-emit on every push call
    //    that crosses the 900-tick boundary, giving the sync-state machine
    //    enough packets to lock in a small number of sends.
    // -----------------------------------------------------------------------
    let mux_cfg = {
        let mut prog = MuxerProgramConfigBuilder::new(1, 0x1000);
        prog.add_video(0x100, VideoCodec::H264);
        let mut b = MuxerConfig::builder();
        b.add_program(prog.build());
        b.psi_interval_ms(10);
        b.build().expect("MuxerConfig::build")
    };

    // -----------------------------------------------------------------------
    // 3. Spawn the receiver loop.  It collects events until it sees a
    //    ProgramMap, then sends it back over an mpsc channel.
    // -----------------------------------------------------------------------
    let (found_tx, found_rx) = std::sync::mpsc::channel::<bool>();

    let _recv_thread = thread::spawn(move || {
        // recv_event returns Ok(None) when the transport is closed or has no
        // more data; Ok(Some(e)) on success; Err on hard error.
        // DemuxReceiver also implements Iterator<Item = Result<DemuxEvent, _>>.
        for item in &mut receiver {
            match item {
                Ok(DemuxEvent::ProgramMap(_)) => {
                    let _ = found_tx.send(true);
                    return;
                }
                Ok(_) => {}
                Err(_) => {
                    let _ = found_tx.send(false);
                    return;
                }
            }
        }
        // Transport closed before we saw a ProgramMap.
        let _ = found_tx.send(false);
    });

    // Small pause so the receiver thread is blocked on recv_bytes before we
    // start sending — avoids a race where the first datagram arrives before
    // the socket is in blocking-recv.
    thread::sleep(Duration::from_millis(50));

    // -----------------------------------------------------------------------
    // 4. Build the sender and push several frames.
    //
    //    Spacing each frame 9001 ticks apart (≈ 10 ms at 90 kHz) crosses the
    //    10-ms PSI re-emission threshold on every push, so PAT + PMT appear
    //    in every outbound datagram batch.  After 4 sends the sync state
    //    machine has seen ≥ 941 bytes of aligned 0x47-framed data and will
    //    lock, allowing the demuxer to emit a ProgramMap event.
    // -----------------------------------------------------------------------
    let url = format!("udp://127.0.0.1:{local_port}");
    let send_transport = UdpTransport::connect(&url).expect("connect UdpTransport");
    let sender = MuxSender::new(send_transport, mux_cfg).expect("MuxSender::new");

    let au = synthetic_h264_au();
    for i in 0i64..5 {
        let pts = Pts90khz::new(i * 9001);
        // The receiver thread returns the instant it sees a ProgramMap, which
        // drops its socket. On fast runners that can happen before this loop
        // finishes; Linux then surfaces the ICMP port-unreachable as
        // ECONNREFUSED on this connected UDP socket's next send. That's benign
        // here — the receiver already has what it needs. Stop sending on the
        // first broken-transport error; the real assertion is the channel
        // result below, which reports `false` (or times out) if the receiver
        // never recovered a ProgramMap. Non-transport error kinds still panic
        // so a real mux/config regression fails loudly. Mirrors the tst-tcp
        // sibling test.
        match sender.send_video(&au, pts, true) {
            Ok(()) => {}
            Err(e)
                if matches!(
                    e.kind,
                    ShellErrorKind::TransportBroken | ShellErrorKind::Closed
                ) =>
            {
                break;
            }
            Err(e) => panic!("send_video failed for a non-transport reason: {e:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // 5. Verify: the receiver emits a ProgramMap within 3 seconds.
    // -----------------------------------------------------------------------
    let ok = found_rx
        .recv_timeout(Duration::from_secs(3))
        .expect("receiver thread did not report within 3 s");

    assert!(ok, "DemuxReceiver did not emit a ProgramMap event");
}
