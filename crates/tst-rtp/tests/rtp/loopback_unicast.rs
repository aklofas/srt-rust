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

/// Find a free RTP port base on `127.0.0.1` where both `base` and `base + 1`
/// (the RTCP companion auto-bound on `port + 1` per RFC 3550 §11) are
/// currently bindable, then release them so the transport under test can bind
/// them itself.
///
/// This replaces the old hard-coded 55004/55008/… ports, which intermittently
/// collided with the Windows dynamic-exclusion / reserved UDP ranges (shift
/// per runner boot) and failed with `WSAEACCES (os error 10013)`. An
/// OS-assigned ephemeral port is never drawn from a reserved range, so this
/// removes that flake class. We can't hand the pre-bound socket to the
/// transport (`RtpRecvTransport::listen` binds by URL and exposes no
/// local-addr accessor), so we discover-then-release; the serialized `network`
/// nextest test-group means no sibling test races for the freed port.
fn free_rtp_port_base() -> u16 {
    use std::net::UdpSocket;
    // Bounded so a host under unusual port pressure fails deterministically with
    // a clear message instead of spinning until nextest's timeout kills it.
    for _ in 0..1000 {
        let s = UdpSocket::bind("127.0.0.1:0").expect("bind ephemeral udp");
        let base = s.local_addr().unwrap().port();
        if base < u16::MAX {
            if let Ok(companion) = UdpSocket::bind(("127.0.0.1", base + 1)) {
                drop(companion);
                drop(s);
                return base;
            }
        }
        drop(s); // base + 1 was taken (or base == u16::MAX); retry.
        std::thread::yield_now();
    }
    panic!(
        "free_rtp_port_base: no free base/base+1 UDP port pair on 127.0.0.1 \
         after 1000 attempts (host under port pressure?)"
    );
}

#[test]
fn unicast_loopback_round_trip() {
    // 1. Bind the receiver first so the sender's send-buffer flushes
    //    immediately. An ephemeral base (with base+1 free for the RTCP
    //    companion auto-bound per Phase 2 Task 10 / RFC 3550 §11) avoids
    //    the Windows reserved-range WSAEACCES flake of fixed ports.
    let base = free_rtp_port_base();
    let url = format!("rtp://127.0.0.1:{base}");
    let mut recv = RtpRecvTransport::listen(&url).unwrap();

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
    let mut send = RtpTransport::connect(&url).unwrap();
    // (RTCP companion: send-side uses an ephemeral local port; recv-
    // side bound base+1 above.)
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
    let base = free_rtp_port_base();
    let recv = RtpRecvTransport::listen(&format!("rtp://127.0.0.1:{base}")).unwrap();
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
    let base = free_rtp_port_base();
    let url = format!("rtp://127.0.0.1:{base}");
    let _recv = RtpRecvTransport::listen(&url).unwrap();
    let mut send = RtpTransport::connect(&url).unwrap();
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
fn rtcp_socket_pair_off_by_default() {
    let base = free_rtp_port_base();
    let r = RtpRecvTransport::listen(&format!("rtp://127.0.0.1:{base}")).unwrap();
    // The experimental SR/RR reporter (and its companion RTCP socket on
    // port+1) is OFF by default (H2). So port+1 must stay free — we can
    // bind it ourselves. (A caller who explicitly opts in via
    // `.rtcp(true)` gets the socket — see `rtcp_socket_pair_opens_when_opted_in`.)
    let probe = std::net::UdpSocket::bind(("127.0.0.1", base + 1));
    assert!(
        probe.is_ok(),
        "RTCP port {} should be free when the reporter is off by default",
        base + 1
    );
    drop(r);
}

#[test]
fn rtcp_opt_out_skips_second_socket() {
    let base = free_rtp_port_base();
    let r = tst_rtp::RtpRecvSocketBuilder::from_url(&format!("rtp://127.0.0.1:{base}"))
        .unwrap()
        .rtcp(false)
        .build()
        .unwrap();
    // With opt-out, port+1 (base+1) stays free.
    let probe = std::net::UdpSocket::bind(("127.0.0.1", base + 1));
    assert!(
        probe.is_ok(),
        "RTCP port {} should be free when builder opts out of RTCP",
        base + 1
    );
    drop(r);
}

#[test]
fn rtcp_socket_pair_opens_when_opted_in() {
    // The experimental reporter is off by default (H2); explicit opt-in
    // must still bind the companion RTCP socket on port+1.
    let base = free_rtp_port_base();
    let r = tst_rtp::RtpRecvSocketBuilder::from_url(&format!("rtp://127.0.0.1:{base}"))
        .unwrap()
        .rtcp(true)
        .build()
        .unwrap();
    // With opt-in, port+1 is held by the transport — binding it ourselves
    // must fail specifically with AddrInUse (any other error would pass
    // spuriously without proving RTCP is bound).
    match std::net::UdpSocket::bind(("127.0.0.1", base + 1)) {
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {}
        other => panic!(
            "RTCP port {} should be held by the opted-in RtpRecvTransport (AddrInUse); got {other:?}",
            base + 1
        ),
    }
    drop(r);
}

#[test]
fn rtcp_stats_accessor_exists() {
    let base = free_rtp_port_base();
    let r = RtpRecvTransport::listen(&format!("rtp://127.0.0.1:{base}")).unwrap();
    let stats = r.rtcp_stats();
    // Counter stays at zero — the experimental RR reporter is off by
    // default (H2), so no reporter thread is spawned and no RR is sent.
    assert_eq!(stats.rr_packets_sent, 0);
    drop(r);
}
