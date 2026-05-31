#![no_std]
#![no_main]
// `loop {}` after `debug::exit` is the no_std unreachable idiom (debug::exit
// returns `()`, so the loop satisfies `fn main() -> !`).
#![allow(clippy::empty_loop)]

extern crate alloc;

use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::mem::MaybeUninit;

use cortex_m_rt::entry;
use cortex_m_semihosting::{debug, hprintln};
use embedded_alloc::Heap;
use panic_semihosting as _;
use spin::Mutex;

use tst_core::mpegts::common::Pts90khz;
use tst_core::mpegts::mux::{Muxer, MuxerConfig, MuxerProgramConfigBuilder, VideoCodec};
use tst_core::transport::{Transport, TransportError};
use tst_pipeline::MuxSender;

#[global_allocator]
static HEAP: Heap = Heap::empty();

const HEAP_SIZE: usize = 256 * 1024;
static mut HEAP_MEM: [MaybeUninit<u8>; HEAP_SIZE] = [MaybeUninit::uninit(); HEAP_SIZE];

/// Committed, CI-guarded golden produced by `gen_scenarios`. Resolved relative
/// to this file: `crates/baremetal-qemu/src/` → `../../tst-integration/...`.
static GOLDEN: &[u8] =
    include_bytes!("../../tst-integration/tests/fixtures/scenarios/video-roundtrip/output.ts");

/// Verbatim copy of `tst_integration::scenarios::synthetic_h264_idr()`.
/// (tst-integration is std-only and not a dependency; per spec decision A the
/// short sequence is duplicated, and the on-device byte-compare detects drift.)
fn synthetic_h264_idr() -> Vec<u8> {
    let mut buf = Vec::with_capacity(20);
    buf.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]); // Annex-B start code
    buf.push(0x65); // nal_ref_idc=11, nal_unit_type=5 (IDR)
    for i in 0u8..15 {
        buf.push(0xA5 ^ i);
    }
    buf
}

/// Verbatim copy of `tst_integration::scenarios::video_roundtrip_ts_bytes()`.
fn video_roundtrip_ts_bytes() -> Vec<u8> {
    let cfg = {
        let mut prog = MuxerProgramConfigBuilder::new(1, 0x1000);
        prog.add_video(0x1011, VideoCodec::H264);
        let mut b = MuxerConfig::builder();
        b.add_program(prog.build());
        b.build().expect("valid muxer config")
    };
    let mut mux = Muxer::new(cfg).expect("muxer init");
    mux.push_video(&synthetic_h264_idr(), Pts90khz::new(0), /*key_frame=*/ true)
        .expect("push_video");

    // Drain (mirrors scenarios::drain_mux: 1316-byte = 7×188 chunks).
    let mut out = Vec::new();
    let mut buf = vec![0u8; 1316];
    loop {
        let n = mux.pull(&mut buf);
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
    }
    out
}

/// Minimal in-memory `Transport`. `Transport: Send` is unconditional, so the
/// single-threaded sink uses `Arc<spin::Mutex<_>>` (Send/Sync) rather than the
/// `!Send` `Rc<RefCell<_>>`. Mirrors the std Task-3 sink in
/// `crates/tst-pipeline/tests/mux_sender_golden.rs`.
/// (Verbatim-local per spec decision A: the no_std binary cannot depend on the
/// std-only tst-integration; the on-device byte-compare detects drift.)
#[derive(Clone)]
struct Sink(Arc<Mutex<Vec<u8>>>);
impl Transport for Sink {
    fn send_bytes(&mut self, b: &[u8]) -> Result<(), TransportError> {
        self.0.lock().extend_from_slice(b);
        Ok(())
    }
    fn max_payload(&self) -> usize {
        1316
    }
    fn close(&mut self) {}
    fn is_alive(&self) -> bool {
        true
    }
}

/// Same config as `video_roundtrip_ts_bytes`, driven through `MuxSender`
/// instead of a bare `Muxer`. Output must equal the same golden.
fn mux_sender_roundtrip_ts_bytes() -> Vec<u8> {
    let cfg = {
        let mut prog = MuxerProgramConfigBuilder::new(1, 0x1000);
        prog.add_video(0x1011, VideoCodec::H264);
        let mut b = MuxerConfig::builder();
        b.add_program(prog.build());
        b.build().expect("valid muxer config")
    };
    let collected = Arc::new(Mutex::new(Vec::new()));
    let sender = MuxSender::new(Sink(Arc::clone(&collected)), cfg).expect("mux sender");
    sender
        .send_video(&synthetic_h264_idr(), Pts90khz::new(0), /*key_frame=*/ true)
        .expect("send_video");
    sender.close();
    // The `let` binding is load-bearing: it drops the `MutexGuard` temporary
    // at end-of-statement, before `collected` itself drops. Inlining as a tail
    // expression extends the guard's borrow past `collected`'s drop (E0597).
    let out = collected.lock().clone();
    out
}

#[entry]
fn main() -> ! {
    unsafe { HEAP.init(core::ptr::addr_of_mut!(HEAP_MEM) as usize, HEAP_SIZE) }

    // Check 1 — bare muxer (P7(a) regression guard).
    let muxer_out = video_roundtrip_ts_bytes();
    if muxer_out != GOLDEN {
        hprintln!(
            "FAIL[muxer]: produced {} bytes, golden {} bytes",
            muxer_out.len(),
            GOLDEN.len()
        );
        debug::exit(debug::EXIT_FAILURE);
        loop {}
    }

    // Check 2 — MuxSender shell over an in-memory transport (P7(b)).
    let sender_out = mux_sender_roundtrip_ts_bytes();
    if sender_out != GOLDEN {
        hprintln!(
            "FAIL[mux_sender]: produced {} bytes, golden {} bytes",
            sender_out.len(),
            GOLDEN.len()
        );
        debug::exit(debug::EXIT_FAILURE);
        loop {}
    }

    hprintln!(
        "PASS: muxer + mux_sender both match golden ({} bytes)",
        GOLDEN.len()
    );
    debug::exit(debug::EXIT_SUCCESS);
    loop {}
}
