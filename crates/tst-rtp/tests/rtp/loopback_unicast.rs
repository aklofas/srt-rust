//! End-to-end RTP loopback over 127.0.0.1.
//!
//! Wires a `tst-rtp` send transport to a `tst-rtp` recv transport on
//! the same host, sends a small synthetic TS payload, and verifies the
//! recv side observes the original bytes after stripping the RTP
//! header.
//!
//! Why two threads: `RtpRecvTransport::recv_bytes` blocks until a
//! datagram arrives or the cancel handle fires; we send from the main
//! thread and recv from a child to avoid deadlocking on a single-thread
//! recv-before-send ordering.

use std::sync::Arc;
use std::thread;
use std::time::Duration;

use tst_core::transport::{RecvTransport, Transport};
use tst_rtp::{RtpRecvTransport, RtpTransport};

/// 188 bytes of arbitrary payload — one MPEG-TS packet's worth.
/// Realistic enough to exercise the wire path without pulling in
/// `tst-pipeline` for this Phase 1 integration test.
fn synthetic_ts_packet(seq_byte: u8) -> [u8; 188] {
    let mut out = [seq_byte; 188];
    out[0] = 0x47; // TS sync byte
    out
}

#[test]
fn unicast_loopback_round_trip() {
    // 1. Bind the receiver first so the sender's send-buffer flushes
    //    immediately.
    //
    // Ports are spaced by 4 (not 1) since Phase 2 Task 10 auto-binds
    // an RTCP companion socket on `port + 1` per RFC 3550 §11. The 4-
    // wide gap leaves room for both RTP + RTCP and a future sender-
    // side companion if we ever symmetrize there too.
    let mut recv = RtpRecvTransport::listen("rtp://127.0.0.1:55004").unwrap();

    // 2. Spawn a recv thread that reads N packets and pushes them to a
    //    channel.
    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
    let _recv_thread = thread::spawn(move || {
        let mut buf = vec![0u8; recv.max_payload() + 64];
        for _ in 0..3 {
            match recv.recv_bytes(&mut buf) {
                Ok(n) => tx.send(buf[..n].to_vec()).unwrap(),
                Err(e) => {
                    eprintln!("recv error: {e:?}");
                    break;
                }
            }
        }
    });

    // 3. Send 3 packets. Tiny sleep lets the OS schedule the recv side
    //    so we don't lose to startup race.
    thread::sleep(Duration::from_millis(20));
    let mut send = RtpTransport::connect("rtp://127.0.0.1:55004").unwrap();
    // (RTCP companion: send-side uses an ephemeral local port; recv-
    // side bound 55005 above. No collision with the next test which
    // uses 55008/55009.)
    let pkts = [
        synthetic_ts_packet(0x01),
        synthetic_ts_packet(0x02),
        synthetic_ts_packet(0x03),
    ];
    for p in &pkts {
        send.send_bytes(p).unwrap();
    }

    // 4. Collect 3 recvs, assert payloads match.
    for expected in pkts.iter() {
        let got = rx.recv_timeout(Duration::from_secs(5)).expect("no recv");
        assert_eq!(got.as_slice(), expected.as_slice());
    }
}

#[test]
fn cancel_wakes_blocked_recv() {
    // 55008 (RTCP at 55009) — leaves the 55004 cohort alone.
    let recv = RtpRecvTransport::listen("rtp://127.0.0.1:55008").unwrap();
    let cancel = recv.cancel_handle().expect("cancel handle");
    let handle = thread::spawn(move || {
        let mut recv = recv;
        let mut buf = vec![0u8; 4096];
        recv.recv_bytes(&mut buf)
    });
    thread::sleep(Duration::from_millis(50));
    cancel.cancel();
    let result = handle.join().unwrap();
    // ExplicitClose is the expected outcome.
    use tst_core::transport::TransportError;
    assert!(matches!(result, Err(TransportError::ExplicitClose)));
}

/// SSRC and seq advance per packet on the send side. Verified by parsing
/// the inbound RTP header from the recv side (which we don't do above —
/// the recv side strips the header before returning). To verify
/// seq-increment we'd need a custom socket bypass, so this test instead
/// confirms the SocketStats counter increments.
#[test]
fn send_stats_increment_per_packet() {
    // 55012 (RTCP at 55013) — leaves the 55004 and 55008 cohorts alone.
    let _recv = RtpRecvTransport::listen("rtp://127.0.0.1:55012").unwrap();
    let mut send = RtpTransport::connect("rtp://127.0.0.1:55012").unwrap();
    let pkt = synthetic_ts_packet(0xFF);
    send.send_bytes(&pkt).unwrap();
    send.send_bytes(&pkt).unwrap();
    send.send_bytes(&pkt).unwrap();
    let stats = send.socket_stats().expect("alive transport");
    assert_eq!(stats.packets_sent, 3);
    assert_eq!(stats.bytes_sent, 3 * (pkt.len() + 12) as u64);
    drop(send);
    let _ = Arc::<()>::default(); // silence unused-arc lint
}

// --- Phase 2 Task 10: RTCP socket-pair retrofit ---
//
// Why these tests live in the existing loopback file: they probe the
// same `listen()` / builder entry points the Phase 1 tests cover. Putting
// them here keeps the per-port test grouping in one spot and avoids a
// separate test binary for three tiny tests.

#[test]
fn rtcp_socket_pair_opens_by_default() {
    let recv_url = "rtp://127.0.0.1:55104";
    let r = RtpRecvTransport::listen(recv_url).unwrap();
    // After Task 10, an RTCP socket is auto-bound on port+1 (55105 here).
    // Probe by trying to bind 55105 ourselves — should fail with AddrInUse.
    let probe = std::net::UdpSocket::bind("127.0.0.1:55105");
    assert!(
        probe.is_err(),
        "RTCP port 55105 should already be bound by RtpRecvTransport"
    );
    drop(r);
}

#[test]
fn rtcp_opt_out_skips_second_socket() {
    let recv_url = "rtp://127.0.0.1:55106";
    let r = tst_rtp::RtpRecvSocketBuilder::from_url(recv_url)
        .unwrap()
        .rtcp(false)
        .build()
        .unwrap();
    // With opt-out, port+1 (55107) is free.
    let probe = std::net::UdpSocket::bind("127.0.0.1:55107");
    assert!(
        probe.is_ok(),
        "RTCP port 55107 should be free when builder opts out of RTCP"
    );
    drop(r);
}

#[test]
fn rtcp_stats_accessor_exists() {
    let r = RtpRecvTransport::listen("rtp://127.0.0.1:55108").unwrap();
    let stats = r.rtcp_stats();
    // Counter starts at zero — the reporter thread won't have fired
    // the first RR yet (RFC 3550 §6.2 randomized interval, 2.5-7.5 s
    // for the 5 s base; tests run faster than that floor).
    assert_eq!(stats.rr_packets_sent, 0);
    drop(r);
}
