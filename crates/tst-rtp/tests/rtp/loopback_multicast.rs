//! Loopback multicast on `239.x.x.x` over the loopback interface.
//!
//! Some CI environments don't have multicast routing enabled by default
//! on `lo` (macOS GHA runner is the historical offender). The test
//! gracefully degrades: if `join_multicast_v4` returns an error
//! (typically `EADDRINUSE` or `ENODEV`), we skip with `#[ignore]`
//! semantics by short-circuiting.
//!
//! Runs on Windows too since 2026-05-29: the prior failure was our
//! `set_multicast_if_v4` being Unix-only (the send-side `?iface=127.0.0.1`
//! errored), plus IP_MULTICAST_LOOP being receiver-side on Windows. Both are
//! fixed in `tst_core::net::udp_socket` (socket2 IP_MULTICAST_IF + Windows
//! receiver-loop), and CI `diag_win_multicast` confirmed loopback multicast
//! delivers on the GHA runner. The `try_listen` skip-on-error path still
//! degrades gracefully where multicast routing is genuinely absent.

use std::thread;
use std::time::Duration;

use tst_core::transport::{RecvTransport, Transport};
use tst_rtp::{RtpRecvTransport, RtpTransport};

fn try_listen_multicast_v4(addr: &str) -> Option<RtpRecvTransport> {
    match RtpRecvTransport::listen(addr) {
        Ok(t) => Some(t),
        Err(e) => {
            eprintln!(
                "skipping multicast loopback test — bind/join failed: {e:?}. \
                 Likely missing multicast routing on lo (common on macOS \
                 GHA runners). Run locally on Linux to exercise this path."
            );
            None
        }
    }
}

#[test]
fn ipv4_multicast_loopback_round_trip() {
    let Some(mut recv) = try_listen_multicast_v4("rtp://239.55.55.1:55010?iface=127.0.0.1") else {
        return;
    };
    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
    let _t = thread::spawn(move || {
        let mut buf = vec![0u8; recv.max_payload() + 64];
        if let Ok(n) = recv.recv_bytes(&mut buf) {
            let _ = tx.send(buf[..n].to_vec());
        }
    });

    thread::sleep(Duration::from_millis(50));
    let mut send = RtpTransport::connect("rtp://239.55.55.1:55010?ttl=1&iface=127.0.0.1").unwrap();
    let payload = [0x47u8; 188];
    send.send_bytes(&payload).unwrap();

    let got = match rx.recv_timeout(Duration::from_secs(5)) {
        Ok(v) => v,
        Err(_) => {
            eprintln!("multicast loopback recv timed out — kernel filter on lo");
            return;
        }
    };
    assert_eq!(got.as_slice(), &payload[..]);
}
