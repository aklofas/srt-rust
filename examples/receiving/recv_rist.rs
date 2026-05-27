//! `tst-rist` quickstart receiver.
//!
//! Binds a RIST receiver on a loopback port and reads packets until
//! interrupted (or until the librist poll-timeout fires too many times,
//! whichever comes first).
//!
//! Why this is teaching code: shows the receiver-bind URL form
//! (`rist://@host:port`) — the `@` follows ffmpeg's convention for
//! "this is a listen address." Calls out the `TransportError::Backpressure`
//! handling that the librist poll-timeout produces between packets.
//!
//! Run with:
//!   cargo run -p tst-examples --example recv_rist
//!
//! Then in another terminal run the matching sender:
//!   cargo run -p tst-examples --example send_rist

use std::error::Error;

use tst_core::transport::{RecvTransport, TransportError};
use tst_rist::{RistProfile, RistRecvTransportBuilder};

const MAX_IDLE_TICKS: u32 = 50; // ~5s of 100ms-poll silence before we bail

fn main() -> Result<(), Box<dyn Error>> {
    // 1. Bind. The `@` prefix marks this as a listen URL (ffmpeg-style).
    //    For encrypted listens, mirror the sender's encryption / profile.
    let url = "rist://@127.0.0.1:9000";
    let mut rx = RistRecvTransportBuilder::new(url)?
        .profile(RistProfile::Simple)
        .listen()?;
    println!(
        "listening on {} (max_payload={})",
        rx.bind_url(),
        rx.max_payload()
    );

    // 2. Read loop. librist's internal poll returns timeout (mapped to
    //    `TransportError::Backpressure`) every 100ms when no data arrives,
    //    so we count consecutive timeouts to bail eventually.
    let mut buf = vec![0u8; rx.max_payload() + 64];
    let mut idle = 0u32;
    let mut total_packets = 0u64;
    let mut total_bytes = 0u64;

    loop {
        match rx.recv_bytes(&mut buf) {
            Ok(n) => {
                idle = 0;
                total_packets += 1;
                total_bytes += n as u64;
                println!(
                    "recv #{:>4}: {n} bytes, first sync={:#04x}, counter={}",
                    total_packets, buf[0], buf[1],
                );
            }
            Err(TransportError::Backpressure { .. }) => {
                idle += 1;
                if idle >= MAX_IDLE_TICKS {
                    eprintln!("no data for {} ticks ({}ms); exiting", idle, idle * 100);
                    break;
                }
            }
            Err(e) => {
                eprintln!("recv error: {e:?}");
                break;
            }
        }
    }

    println!("total: {total_packets} packets, {total_bytes} bytes");
    Ok(())
}
