//! Live-socket end-to-end roundtrip — `pipeline::MuxSender` → real SRT loopback
//! → `pipeline::DemuxReceiver` in one process.
//!
//! Why a separate test from `pipeline_receiver.rs` (canned-transport) and
//! `pipeline_sender.rs` (live socket, raw byte counter)?
//!
//! - `pipeline_receiver.rs` exercises the receiver composition with
//!   `CannedTransport`, which replays in-memory chunks and never goes near
//!   the SRT handshake or the wire.
//! - `pipeline_sender.rs` exercises the sender composition over a real
//!   `srt::Listener` ↔ `srt::Socket` pair, but the receive side just counts
//!   raw bytes — it never demuxes.
//!
//! This test wires the two halves together. It validates that the sender
//! pipeline's wire format survives a full SRT handshake, transit, and
//! reassembly, and that the receiver pipeline produces semantically
//! correct events on the other side.
//!
//! Linux x86_64 only — same gate as the existing live-socket tests.

#![cfg(target_os = "linux")]

mod common;

use common::synthetic_nal;
use tst_core::mpegts::demux::DemuxEvent;
use tst_core::mpegts::mux::{ConfigBuilder, KlvStreamType, VideoCodec as MuxVideoCodec};
use tst_pipeline::{DemuxReceiver, DemuxReceiverError, MuxSender, TransportError};
use tst_srt::SrtTransport;
use tst_srt::{ListenerBuilder, SocketBuilder};
use std::thread;
use std::time::Duration;

/// Minimal KLV blob with a valid SMPTE UL prefix so the demuxer classifies
/// it as `MetadataKind::KlvAsync`. Mirrors `minimal_klv()` in
/// `tests/pipeline_receiver.rs`. A bare ASCII placeholder would land in
/// `MetadataKind::Unknown` instead — counts toward `metas` only because we
/// don't filter by kind, but we want the right kind to flow through.
fn minimal_klv() -> Vec<u8> {
    // 16-byte SMPTE UL for ST 0601 + BER-short length + minimal body
    // (UDS tag 2 = "UAS LS Version Number" at 8 zero bytes — content is
    // semantically nonsense but the wrapper is well-formed).
    let body: &[u8] = &[2u8, 8, 0, 0, 0, 0, 0, 0, 0, 0];
    let mut out = Vec::with_capacity(17 + body.len());
    out.extend_from_slice(&[
        0x06, 0x0E, 0x2B, 0x34, 0x02, 0x0B, 0x01, 0x01, 0x0E, 0x01, 0x03, 0x01, 0x01, 0x00, 0x00,
        0x00,
    ]);
    out.push(body.len() as u8);
    out.extend_from_slice(body);
    out
}

/// Number of video AUs and KLV blobs the sender pushes. We assert
/// `samples >= EXPECT` and `metas >= EXPECT` — sender pushes a few extra
/// frames so the early-break condition trips well before the close.
///
/// Why the buffer? When libsrt signals peer close as `Broken` (not the
/// graceful `Closed`), `DemuxReceiver` propagates the error before calling
/// `Demuxer::flush()`. The last in-flight video AU only completes when
/// the *next* PUSI arrives (H.264 PES with length=0 sentinel), so without
/// flush the final AU is lost. Sending `SEND - EXPECT = 5` extra frames
/// makes sure we comfortably cross the assertion threshold while events
/// are still streaming through the queue.
const SEND: usize = 15;
const EXPECT: usize = 10;

#[test]
fn end_to_end_sender_to_receiver() {
    // Listener side: bind to ephemeral port. `recv_latency` budget gives
    // libsrt's TSBPD path time to reorder + emit packets even on a busy CI
    // box; matches the sender side's `latency` for symmetry.
    let mut listener = ListenerBuilder::new()
        .recv_latency(Duration::from_millis(120))
        .bind("127.0.0.1:0")
        .expect("bind listener");
    let port = listener.local_addr().unwrap().port();

    // MuxSender thread: connect, build the pipeline, push N video + N KLV
    // frames at 30 fps PTS spacing (3000 ticks ≈ 33 ms at 90 kHz), then
    // brief sleep + close to let bytes drain on the wire.
    let send_handle = thread::spawn(move || {
        let socket = SocketBuilder::new()
            .latency(Duration::from_millis(120))
            .connect(format!("127.0.0.1:{port}"))
            .expect("connect");

        // Two-stream PMT: H.264 video on PID 0x100, async KLV on PID 0x101.
        // `add_klv(.., PrivateData, false)` matches the demuxer's async-KLV
        // recognition path — `false` means the muxer doesn't emit a PTS on
        // the KLV PES (typical for low-rate metadata).
        let cfg = ConfigBuilder::default()
            .add_program(1, 0x1000)
            .add_video(0x100, MuxVideoCodec::H264)
            .add_klv(0x101, KlvStreamType::PrivateData, false)
            .end_program()
            .build()
            .expect("build mux config");
        let sender = MuxSender::new(cfg, SrtTransport::new(socket)).expect("sender");

        let klv = minimal_klv();
        for i in 0..SEND as i64 {
            // Realistic AU body (500 bytes) with synthetic NAL header.
            // Frame 0 is a key frame; rest are P-frames. The mux treats
            // bytes opaquely so the only thing that matters wire-side is
            // start-code framing, which `synthetic_nal::h264_au` produces.
            let key = i == 0;
            let nal = synthetic_nal::h264_au(500, key);
            let pts = i * 3_000;
            sender.send_video(&nal, pts, key).expect("send_video");
            sender.send_klv(&klv, pts).expect("send_klv");
        }

        // Brief drain pause: SRT's send queue is async w.r.t. close, so
        // closing immediately can drop in-flight packets before TSBPD on
        // the peer releases them. 200 ms covers typical loopback latency
        // plus the 120 ms latency budget.
        thread::sleep(Duration::from_millis(200));
        sender.close();
    });

    // DemuxReceiver side: accept, wrap in SrtTransport, drive the DemuxReceiver.
    // `accept` blocks until the sender's connect completes the handshake.
    let (server_socket, _peer) = listener.accept().expect("accept");
    let mut rx = DemuxReceiver::new(SrtTransport::new(server_socket));

    let mut samples = 0usize;
    let mut metas = 0usize;
    let mut got_pmap = false;

    // Drain events. Two valid termination paths:
    //   1. Iterator returns `None` — clean EOF after `Closed` triggered the
    //      demuxer's tail flush.
    //   2. Iterator returns `Some(Err(Transport(Broken(_))))` — peer hangup.
    //      libsrt typically signals sender-side close as a Broken receive on
    //      the peer (see `srt_transport.rs:112` mapping). Treat that as a
    //      clean stream end here: the sender did its job and any events
    //      already queued in the demuxer have been delivered.
    //
    // Early-exit once we've counted enough — covers both paths. A `Demux`
    // error or any non-Broken transport error is a real bug; fail loudly.
    for item in &mut rx {
        let event = match item {
            Ok(e) => e,
            Err(DemuxReceiverError::Transport(TransportError::Broken(_))) => break,
            Err(other) => panic!("unexpected receiver error: {other:?}"),
        };
        match event {
            DemuxEvent::ProgramMap(_) => got_pmap = true,
            DemuxEvent::Sample { .. } => samples += 1,
            DemuxEvent::Metadata { .. } => metas += 1,
            // Discontinuity / NonConformant aren't expected on a clean
            // loopback round-trip but aren't fatal — let them pass.
            _ => {}
        }
        if samples >= EXPECT && metas >= EXPECT {
            break;
        }
    }

    send_handle.join().expect("send thread");

    assert!(got_pmap, "receiver should have observed PMT");
    assert!(
        samples >= EXPECT,
        "expected ≥ {EXPECT} video samples; got {samples}"
    );
    assert!(
        metas >= EXPECT,
        "expected ≥ {EXPECT} metadata events; got {metas}"
    );
}
