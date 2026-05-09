//! Hello, MPEG-TS + KLV.
//!
//! The smallest possible round-trip showing what this library does:
//! build one TS frame containing one video access unit and one
//! ST 0601 KLV record, in memory.
//!
//!   cargo run -p tst-examples --example hello_world
//!
//! No SRT, no files, no real codecs — just bytes in, bytes out.
//! From here:
//!   - To send over SRT:                 sending/pipeline_send_to_socket.rs
//!   - To write to a real .ts file:      muxing/mux_to_file.rs
//!   - To carry real H.265 video:        muxing/mux_h265_with_klv.rs
//!   - To decode the KLV blob back:      klv-metadata/klv_decode_file.rs

use tst_core::klv::st0601::{UasDatalinkLs, encode_to_vec};
use tst_core::mpegts::mux::{KlvStreamType, Muxer, MuxerConfig, VideoCodec};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Build a muxer: one video + one async-KLV stream on one program.
    //    PIDs are 13-bit identifiers in 0x0010..=0x1FFE; 0x100/0x101 are
    //    arbitrary unused values. async-KLV stream_type 0x06 (private data)
    //    is the simplest KLV mode — payload passes through verbatim, no
    //    AU cell wrap and no per-record PTS (`carries_pts: false`).
    let config = MuxerConfig::builder()
        .add_program(/*program_number=*/ 1, /*pmt_pid=*/ 0x1000)
        .add_video(/*pid=*/ 0x100, VideoCodec::H264)
        .add_klv(
            /*pid=*/ 0x101,
            KlvStreamType::PrivateData,
            /*carries_pts=*/ false,
        )
        .end_program()
        .build()?;
    let mut mux = Muxer::new(config)?;

    // 2. Push one synthetic AUD (Access Unit Delimiter) NAL with Annex-B
    //    start code. Real callers feed encoder output; we hand-roll one
    //    byte string here so the example has no codec dependency.
    let aud_nal = &[0x00, 0x00, 0x00, 0x01, 0x09, 0x10];
    mux.push_video(aud_nal, /*pts_90khz=*/ 0, /*key_frame=*/ true)?;

    // 3. Push one ST 0601 KLV record (just a UTC timestamp tag — Tag 2).
    let ls = UasDatalinkLs {
        timestamp_us: Some(0),
        ..UasDatalinkLs::default()
    };
    let klv_bytes = encode_to_vec(&ls)?;
    mux.push_klv(&klv_bytes, /*pts_90khz=*/ 0, /*metadata_service_id=*/ 0)?;

    // 4. Drain TS packets out of the muxer. Muxer::pull writes 188 bytes
    //    at a time into a caller-provided buffer; returns 0 when empty.
    let mut packet = [0u8; 188];
    let mut total = 0;
    loop {
        let n = mux.pull(&mut packet);
        if n == 0 {
            break;
        }
        total += n;
    }
    println!(
        "Built {} bytes of MPEG-TS ({} packets) containing 1 video AU + 1 KLV record.",
        total,
        total / 188,
    );
    println!("Next: muxing/mux_to_file.rs writes a real .ts you can play with ffprobe.");
    Ok(())
}
