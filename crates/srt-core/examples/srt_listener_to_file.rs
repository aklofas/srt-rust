//! Minimal SRT receiver: bind, accept one connection, write bytes to a file.
//!
//! The peer for the existing `pipeline_send_to_socket` and `ts_relay_from_file`
//! examples — run this in one terminal, run the sender in another:
//!
//!   # terminal A (this example)
//!   cargo run --example srt_listener_to_file -- 127.0.0.1:9000 out.ts
//!
//!   # terminal B (the sender)
//!   cargo run --example pipeline_send_to_socket -- 127.0.0.1:9000
//!
//! Stops after the first sender disconnects.

use srt_core::srt::ListenerBuilder;
use std::env;
use std::fs::File;
use std::io::Write;
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    let bind_addr = args.next().unwrap_or_else(|| "127.0.0.1:9000".into());
    let out_path = args.next().unwrap_or_else(|| "out.ts".into());

    let mut listener = ListenerBuilder::new()
        .latency(Duration::from_millis(120))
        .bind(bind_addr.as_str())?;

    eprintln!("listening on {bind_addr}, writing to {out_path}");
    let (mut socket, peer) = listener.accept()?;
    eprintln!("accepted from {peer}");

    let mut out = File::create(&out_path)?;
    let mut buf = [0u8; 1500];
    let mut total: u64 = 0;
    loop {
        match socket.recv(&mut buf) {
            Ok(n) => {
                out.write_all(&buf[..n])?;
                total += n as u64;
                if total % (256 * 1024) < n as u64 {
                    eprintln!("received {total} bytes");
                }
            }
            Err(srt_core::error::RecvError::ConnectionBroken) => {
                eprintln!("peer closed; received {total} bytes total");
                break;
            }
            Err(srt_core::error::RecvError::TimedOut) => continue,
            Err(e) => return Err(Box::new(e)),
        }
    }
    Ok(())
}
