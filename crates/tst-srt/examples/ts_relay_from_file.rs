//! Example: read a `.ts` file → `pipeline::Sender` → connected SRT socket.
//!
//! Run with:
//!   cargo run --example ts_relay_from_file -- input.ts 127.0.0.1:9000

use tst_pipeline::{ Sender, SenderConfig};
use tst_srt::SrtTransport;
use tst_srt::SocketBuilder;
use std::env;
use std::fs::File;
use std::io::Read;
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    let path = args
        .next()
        .ok_or("usage: ts_relay_from_file <file.ts> <addr>")?;
    let addr = args.next().ok_or("missing addr")?;

    let socket = SocketBuilder::new()
        .latency(Duration::from_millis(120))
        .connect(addr.as_str())?;
    let transport = SrtTransport::new(socket);
    let mut sender = Sender::new(transport, SenderConfig::default());

    let mut file = File::open(&path)?;
    let mut buf = vec![0u8; 4096];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        sender.send_ts(&buf[..n])?;
    }
    sender.flush()?;
    eprintln!("relayed {path} -> {addr}");
    eprintln!("stats: {:?}", sender.stats());
    sender.close();
    std::thread::sleep(Duration::from_millis(200));
    Ok(())
}
