//! Regression test for the partial-write-then-WouldBlock silent-corruption bug.
//!
//! Under a stalled peer the kernel send buffer fills. A single `send_bytes`
//! message can be partially committed to the wire, after which the next
//! internal `write()` hits the 100ms write timeout and returns `WouldBlock`.
//! Mapping that to `Backpressure` would let the MuxSender retry path re-send
//! the identical chunk, duplicating the already-sent prefix and desyncing the
//! receiver's 188-byte TS framing (silent corrupted video + KLV).
//!
//! The contract (tst_core::transport::Transport::send_bytes): only a
//! *zero-progress* WouldBlock may be reported as `Backpressure`. A partial
//! prefix already on the wire must be reported as `Broken` and the transport
//! marked dead so the caller rebuilds instead of re-sending.

use std::net::TcpListener as StdTcpListener;
use std::thread;
use std::time::Duration;

use tst_core::transport::{Transport, TransportError};
use tst_tcp::url::TcpUrl;
use tst_tcp::{SocketConfig, TcpTransport};

/// Build a caller-side TcpTransport with a tiny SO_SNDBUF and a large per-call
/// pkt_size, so a single big `send_bytes` straddles the kernel send buffer.
fn connect_tiny_sndbuf(port: u16) -> TcpTransport {
    let url = TcpUrl::parse(&format!("tcp://127.0.0.1:{port}")).unwrap();
    // SocketConfig is #[non_exhaustive] → build via Default then assign fields.
    let mut cfg = SocketConfig::default();
    // Small send buffer → fills fast so a partial commit is forced.
    cfg.sndbuf = Some(2048);
    // Large per-call cap so a single message can exceed the buffer space.
    cfg.pkt_size = Some(4 * 1024 * 1024);
    TcpTransport::connect_with_config(&url, &cfg).unwrap()
}

/// A stalled peer that accepts but NEVER reads forces the send buffer to fill.
/// Sending TS-sized payloads in a loop must eventually return a non-Ok result;
/// once a *partial* commit occurs it MUST be `Broken` (never `Backpressure`),
/// and the transport MUST then report `is_alive() == false`.
#[test]
fn partial_write_then_wouldblock_is_broken_not_backpressure() {
    let listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();

    // Accept the connection but never read — also pin a tiny receive buffer so
    // the total in-flight window (sndbuf + peer rcvbuf) is small. Hold the
    // accepted socket for the whole test so it is not dropped/closed early.
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<()>();
    let peer = thread::spawn(move || {
        let (sock, _) = listener.accept().unwrap();
        let s = socket2::SockRef::from(&sock);
        let _ = s.set_recv_buffer_size(2048);
        ready_tx.send(()).unwrap();
        // Park forever (until the test ends) without ever reading.
        thread::sleep(Duration::from_secs(30));
        drop(sock);
    });

    let mut send = connect_tiny_sndbuf(port);
    ready_rx.recv_timeout(Duration::from_secs(2)).unwrap();

    // A 256 KiB message is far larger than the ~few-KiB in-flight window, so a
    // single send_bytes call must partially commit then time out → Broken.
    let big = vec![0x47u8; 256 * 1024];

    // Loop a bounded number of times; the very first call already straddles the
    // buffer, but loop to be robust to scheduling. We must observe a Broken
    // (partial-progress) result, never a Backpressure on a partial.
    let mut saw_broken = false;
    for _ in 0..16 {
        match send.send_bytes(&big) {
            Ok(()) => continue,
            Err(TransportError::Backpressure { .. }) => {
                // Zero-progress backpressure is permitted by the contract, but
                // for a 256 KiB message against a ~4 KiB window this should not
                // happen before a partial commit. Keep trying.
                continue;
            }
            Err(TransportError::Broken { msg, .. }) => {
                assert!(
                    msg.contains("partial write") || msg.contains("desynced"),
                    "expected partial-write Broken message, got: {msg}"
                );
                saw_broken = true;
                break;
            }
            Err(other) => panic!("unexpected error: {other:?}"),
        }
    }

    assert!(
        saw_broken,
        "expected a Broken result from a partial write against a stalled peer"
    );
    // After a partial-write Broken, the transport must be dead so the caller
    // rebuilds rather than re-sending onto a desynced stream.
    assert!(
        !send.is_alive(),
        "transport must be marked dead after a partial-write Broken"
    );

    // Let the peer thread finish (it sleeps; just detach — test is done).
    drop(peer);
}
