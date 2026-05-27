//! `tst-rist` quickstart sender.
//!
//! Demonstrates the smallest useful RIST send path: build a sender from a
//! URL, send a handful of MPEG-TS-looking packets through it.
//!
//! Why this is teaching code: the URL shape (`rist://host:port` /
//! `rist://@host:port` / `?profile=main&aes-type=256&secret=...`) mirrors
//! ffmpeg's RIST URL syntax. The comments call out where RIST differs from
//! UDP (recovery buffer + RTCP + optional AES PSK) and where it differs
//! from SRT (no caller/listener handshake, simpler peer model).
//!
//! Run with:
//!   cargo run -p tst-examples --example send_rist
//!
//! There's no receiver here; the kernel happily sends UDP datagrams into
//! the void. To see the bytes, run `recv_rist` (`cargo run -p tst-examples
//! --example recv_rist`) in another terminal first.

use std::error::Error;
use std::thread;
use std::time::Duration;

use tst_core::transport::Transport;
use tst_rist::{RistProfile, RistTransportBuilder};

fn main() -> Result<(), Box<dyn Error>> {
    // 1. Build a Simple Profile sender. For unencrypted use the Simple
    //    profile is sufficient and avoids the longer handshake of Main
    //    Profile. To switch to Main + AES-256:
    //
    //       .profile(RistProfile::Main)
    //       .encryption(EncryptionKey::aes256("psk-here"))
    //
    //    librist's handshake takes ~500ms (Simple) or ~1s (Main+AES) —
    //    if your application is latency-critical, build the sender once
    //    and reuse it.
    let url = "rist://127.0.0.1:9000";
    let mut tx = RistTransportBuilder::new(url)?
        .profile(RistProfile::Simple)
        .buffer(Duration::from_millis(200))
        .connect()?;
    println!(
        "connected to {} (max_payload={})",
        tx.peer_url(),
        tx.max_payload()
    );

    // 2. Give librist a moment to settle. Without this the first ~2-3
    //    packets often get dropped on the floor while the session
    //    handshake is still negotiating.
    thread::sleep(Duration::from_millis(600));

    // 3. Send 100 hand-built TS-looking packets. Pattern picked so that
    //    `tcpdump -i lo udp port 9000 -X` makes the packet boundaries
    //    visually obvious. Real callers use `MuxSender<RistTransport>`
    //    and push H.264 / H.265 / KLV / etc.; the raw `Transport::
    //    send_bytes` path is shown here for clarity.
    for i in 0u8..100 {
        let mut pkt = [0u8; 188];
        pkt[0] = 0x47; // TS sync byte
        pkt[1] = i; // identifying counter
        pkt[2..].fill(0xab);
        tx.send_bytes(&pkt)?;
    }

    // 4. Stats snapshot — bytes_sent + packets_sent cumulative counters.
    let stats = tx.stats();
    println!(
        "sent: {} packets, {} bytes",
        stats.packets_sent, stats.bytes_sent
    );

    Ok(())
}
