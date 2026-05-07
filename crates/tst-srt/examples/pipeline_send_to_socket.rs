//! Example: synthetic frames → `pipeline::MuxSender` → connected SRT socket.
//!
//! Run with:
//!   cargo run --example pipeline_send_to_socket -- 127.0.0.1:9000
//!
//! On the receiver side:
//!   srt-live-transmit srt://:9000 file:///tmp/out.ts

use std::env;
use std::time::Duration;
use tst_core::mpegts::mux::Config;
use tst_pipeline::MuxSender;
use tst_srt::SocketBuilder;
use tst_srt::SrtTransport;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:9000".into());

    let socket = SocketBuilder::new()
        .latency(Duration::from_millis(120))
        .connect(addr.as_str())?;
    let transport = SrtTransport::new(socket);
    let sender = MuxSender::new(Config::default(), transport)?;

    eprintln!("sending 5 synthetic frames + KLV to {addr}");
    for i in 0..5 {
        let nal = synthetic_nal_au(500);
        let klv = synthetic_klv(64, i);
        sender.send_video(&nal, i * 33_000, i == 0)?;
        // `metadata_service_id` goes into the AU cell header per H.222.0
        // §2.12.4.2 / ST 1402.2 App. B Table 2 for SynchronousMetadata
        // streams (stream_type 0x15); silently ignored for PrivateData
        // streams (0x06) like the one used here. The spec default is 0x00.
        sender.send_klv(&klv, (i * 33_000) * 90 / 1000, 0x00)?;
        std::thread::sleep(Duration::from_millis(33));
    }
    eprintln!("done. closing.");
    sender.close();
    std::thread::sleep(Duration::from_millis(200));
    Ok(())
}

fn synthetic_nal_au(payload_size: usize) -> Vec<u8> {
    let mut buf = vec![0x00, 0x00, 0x00, 0x01, 0x65]; // start code + IDR
    buf.extend(std::iter::repeat(0xAA).take(payload_size));
    buf
}

fn synthetic_klv(size: usize, seq: i64) -> Vec<u8> {
    // Minimal ST 0601: UL + BER length + payload. Use a placeholder UL
    // for example purposes only.
    let mut buf = vec![
        0x06, 0x0E, 0x2B, 0x34, 0x02, 0x0B, 0x01, 0x01, 0x0E, 0x01, 0x03, 0x01, 0x01, 0x00, 0x00,
        0x00,
    ];
    buf.push(size as u8); // BER short-form length
    buf.extend(std::iter::repeat(seq as u8).take(size));
    buf
}
