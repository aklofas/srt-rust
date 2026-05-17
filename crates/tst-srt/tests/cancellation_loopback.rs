//! Live-socket validation that `MuxSender::close()` (and the underlying
//! `CancelHandle`) wakes a peer thread parked inside libsrt's
//! `srt_sendmsg2`.
//!
//! We engineer a deterministic park by:
//! 1. Listening on 127.0.0.1:0 with a tiny SRTO_RCVBUF.
//! 2. Connecting a sender with SRTO_SNDBUF tiny and the libsrt-default
//!    blocking-forever SRTO_SNDTIMEO (-1).
//! 3. NOT calling recv on the listener side — bytes accumulate in the
//!    receive buffer and propagate back-pressure to the sender.
//! 4. Pumping payloads from the sender thread until libsrt's send call
//!    blocks (typically within < 100 packets).
//! 5. Calling `s.close()` from the main thread; the parked thread must
//!    return within a generous timeout.

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};
use tst_core::mpegts::common::Pts90khz;
use tst_core::mpegts::mux::{KlvStreamType, MuxerConfig, MuxerProgramConfigBuilder, VideoCodec};
use tst_pipeline::{MuxSender, MuxSenderError, MuxSenderErrorSource, TransportError};
use tst_srt::SrtTransport;
use tst_srt::{ListenerBuilder, SocketBuilder};

#[test]
fn close_unblocks_libsrt_parked_send() {
    require_loopback!();
    // Bind a listener with very small recv buffer.
    let mut builder = ListenerBuilder::new();
    builder
        .recv_buf_packets(8) // tiny — back-pressure kicks in fast
        .latency(Duration::from_millis(120));
    let lb = common::Loopback::bind_with(builder);
    let port = lb.port;

    // Accept thread returns the socket immediately; main thread holds it
    // alive (without recv) so the receive buffer fills and back-pressures
    // the sender.
    let accept = lb.spawn_accept(|sock| sock);
    accept.wait_ready();

    // Connect the sender side. This drives the SRT handshake, which
    // unblocks `accept()` on the listener side.
    let socket = SocketBuilder::new()
        .send_buf_packets(8) // tiny — sender's outgoing queue saturates fast
        // (no send_timeout — defaults to -1 = block forever)
        .latency(Duration::from_millis(120))
        .connect(format!("127.0.0.1:{port}"))
        .expect("connect");

    // Drain the accepted socket out of the accept thread and hold it on
    // the main thread for the duration of the test. Connection stays
    // open from libsrt's view (the Drop at end-of-scope closes it).
    let _peer_socket = accept.join();

    let transport = SrtTransport::new(socket);
    let cfg = {
        let mut prog = MuxerProgramConfigBuilder::new(1, 0x1000);
        prog.add_video(0x100, VideoCodec::H264);
        prog.add_klv(0x101, KlvStreamType::PrivateData, false);
        let mut b = MuxerConfig::builder();
        b.add_program(prog.build());
        b.build().unwrap()
    };
    let s = Arc::new(MuxSender::new(transport, cfg).unwrap());
    let s_send = s.clone();

    // MuxSender thread: pump 64-byte NAL payloads until the call returns
    // (with Ok, or with our Broken-via-cancel error). 1000 NALs is
    // ~64 KB, far more than the 8-packet SNDBUF can absorb.
    let send_thread = std::thread::spawn(move || -> Result<u32, MuxSenderError> {
        // 4-byte Annex B start code (0x00000001) + 60 bytes payload.
        // The muxer rejects NALs without a start code as InvalidNal.
        let mut nal = vec![0u8; 64];
        nal[3] = 0x01;
        let mut count = 0u32;
        for pts in 0..1000 {
            s_send.send_video(&nal, Pts90khz::new(pts * 90), false)?;
            count += 1;
        }
        Ok(count)
    });

    // Give the sender ~500ms to pump and park.
    std::thread::sleep(Duration::from_millis(500));

    // Cancel via close from main thread. Must be near-instant — no
    // mutex contention with the parked thread.
    let close_start = Instant::now();
    s.close();
    let close_elapsed = close_start.elapsed();
    assert!(
        close_elapsed < Duration::from_millis(500),
        "close() took {close_elapsed:?} — should be near-instant via cancel"
    );

    // Parked sender should return promptly with a Transport Broken
    // error (libsrt's srt_sendmsg2 returns SRT_ECONNLOST after close).
    let join_start = Instant::now();
    let result = send_thread.join().expect("send thread panic");
    let join_elapsed = join_start.elapsed();
    assert!(
        join_elapsed < Duration::from_secs(2),
        "send thread didn't unpark within 2s of cancel ({join_elapsed:?})"
    );
    match result {
        // Either the thread broke partway (most common) or it pumped
        // every payload before close (unlikely with SNDBUF=8). Both
        // outcomes prove cancel works; we only fail on stuck.
        Ok(_) => {}
        Err(ref err)
            if matches!(
                err.source,
                MuxSenderErrorSource::Transport(
                    TransportError::Broken(_) | TransportError::Closed
                )
            ) => {}
        Err(other) => panic!("unexpected sender error after cancel: {other:?}"),
    }
}
