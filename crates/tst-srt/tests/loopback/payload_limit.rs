//! Verifies `PayloadTooLarge` reports the actual configured limit, not 1316.
//! Regression for audit Issue 5.

use std::time::Duration;
use tst_core::transport::Transport;
use tst_srt::error::SendError;
use tst_srt::{ListenerBuilder, SocketBuilder, SrtTransport};

#[test]
fn payload_too_large_reports_configured_limit() {
    require_loopback!();
    let mut builder = ListenerBuilder::new();
    builder
        .payload_size(1456)
        .recv_timeout(Duration::from_secs(5));
    let lb = crate::common::Loopback::bind_with(builder);
    let port = lb.port;

    let accept = lb.spawn_accept(|sock| sock);
    accept.wait_ready();

    let mut sender = SocketBuilder::new()
        .payload_size(1456)
        .send_timeout(Duration::from_secs(5))
        .connect(("127.0.0.1", port))
        .expect("connect");

    let _accepted = accept.join();

    let big = vec![0u8; 2000];
    match sender.send(&big) {
        Err(SendError::PayloadTooLarge { actual, limit }) => {
            assert_eq!(actual, 2000);
            assert_eq!(
                limit, 1456,
                "limit should be configured payload size, not the libsrt default"
            );
        }
        other => panic!("expected PayloadTooLarge {{ limit: 1456 }}, got {other:?}"),
    }
}

/// Regression for validate-1 C1 (Codex SRT-01): `SrtTransport::new`
/// previously hardcoded `max_payload = 1316`, so a socket configured
/// with `SRTO_PAYLOADSIZE = 1456` would be silently truncated by the
/// transport-level pre-check at `send_bytes` — and on the receiver
/// side a buffer sized from `transport.max_payload()` would be too
/// small for the 1456-byte messages libsrt actually delivers
/// (`BufferTooSmall` → `Broken`).
///
/// This test wires up a real loopback handshake with `payloadsize=1456`
/// on both peers, hands the connected `Socket` to `SrtTransport::new`,
/// and asserts the transport reports 1456 — proving the negotiated
/// value flows through to the `Transport::max_payload()` contract.
#[test]
fn srt_transport_inherits_configured_payload_size() {
    require_loopback!();
    let mut builder = ListenerBuilder::new();
    builder
        .payload_size(1456)
        .recv_timeout(Duration::from_secs(5));
    let lb = crate::common::Loopback::bind_with(builder);
    let port = lb.port;

    // Accept side: wrap the accepted Socket in SrtTransport and report
    // its max_payload() back via the join-channel.
    let accept = lb.spawn_accept(|sock| {
        let transport = SrtTransport::new(sock);
        Transport::max_payload(&transport)
    });
    accept.wait_ready();

    let socket = SocketBuilder::new()
        .payload_size(1456)
        .send_timeout(Duration::from_secs(5))
        .connect(("127.0.0.1", port))
        .expect("connect");

    // Sender side: wrap the connected Socket the same way the C ABI
    // `connect_srt` helper does, and check the transport reports 1456.
    let sender_transport = SrtTransport::new(socket);
    assert_eq!(
        Transport::max_payload(&sender_transport),
        1456,
        "SrtTransport::new must derive max_payload from socket, not hardcode 1316"
    );

    let accepted_max = accept.join();
    assert_eq!(
        accepted_max, 1456,
        "accepted-side SrtTransport must also derive max_payload from socket"
    );
}

/// Sibling sanity: when a socket is built with no `payload_size` config
/// (libsrt default), `SrtTransport::new` reports the
/// `SRT_TS_BUNDLE_BYTES` (1316) constant — preserving prior behavior
/// for the unconfigured path.
#[test]
fn srt_transport_default_payload_when_unconfigured() {
    require_loopback!();
    let lb = crate::common::Loopback::bind();
    let port = lb.port;

    let accept = lb.spawn_accept(|sock| {
        let t = SrtTransport::new(sock);
        Transport::max_payload(&t)
    });
    accept.wait_ready();

    let socket = SocketBuilder::new()
        .send_timeout(Duration::from_secs(5))
        .connect(("127.0.0.1", port))
        .expect("connect");

    let t = SrtTransport::new(socket);
    assert_eq!(Transport::max_payload(&t), SrtTransport::DEFAULT_PAYLOAD);

    let accepted_max = accept.join();
    assert_eq!(accepted_max, SrtTransport::DEFAULT_PAYLOAD);
}
