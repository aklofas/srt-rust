//! S4 host harness: a tst-srt SRT listener that receives the firmware caller's
//! GOLDEN×N stream over SLIRP and verifies it byte-exact. The authoritative
//! verdict for the S4 gate. `S4_PASSPHRASE` env (set by the gate in Phase B)
//! enables AES-128.
use std::io::Write;
use std::process::exit;
use tst_srt::{KeyLength, ListenerBuilder, Passphrase};

include!(concat!(env!("OUT_DIR"), "/golden.rs")); // pub static GOLDEN: [u8;564]

const REPEAT: usize = 64;
const STREAM_LEN: usize = 564 * REPEAT;

fn main() {
    let aes = std::env::var("S4_PASSPHRASE").ok();
    let tag = if aes.is_some() { "s4_host_aes" } else { "s4_host_plain" };

    let mut lb = ListenerBuilder::new();
    // LIVE mode (tst-srt default) — matches the firmware caller's SRTT_LIVE.
    if let Some(pass) = aes.as_deref() {
        lb.passphrase(Passphrase::new(pass.to_string()).expect("valid passphrase"));
        lb.key_length(KeyLength::Aes128);
    }
    let mut listener = match lb.bind("0.0.0.0:9000") {
        Ok(l) => l,
        Err(e) => { eprintln!("FAIL[{tag}]: bind: {e}"); exit(1); }
    };

    // Signal the gate that the bind is up BEFORE the caller is launched.
    println!("host-ready");
    let _ = std::io::stdout().flush();

    let (mut sock, _peer) = match listener.accept() {
        Ok(x) => x,
        Err(e) => { eprintln!("FAIL[{tag}]: accept: {e}"); exit(1); }
    };

    // LIVE/message mode: srt_recv requires the buffer to be at least the
    // negotiated payload size (~1456 B), even though each message is only 564 B
    // — passing a smaller tail slice throws "Incorrect use of Message API". So
    // recv into a fixed full-size temp buffer each call, then copy into place.
    let mut buf = vec![0u8; STREAM_LEN];
    let mut tmp = [0u8; 2048];
    let mut got = 0usize;
    while got < STREAM_LEN {
        match sock.recv(&mut tmp) {
            Ok(0) => break,
            Ok(n) => {
                let take = n.min(STREAM_LEN - got);
                buf[got..got + take].copy_from_slice(&tmp[..take]);
                got += take;
            }
            Err(e) => { eprintln!("FAIL[{tag}]: recv at {got}: {e}"); exit(1); }
        }
    }

    let len_ok = got == STREAM_LEN;
    let bytes_ok = len_ok && (0..REPEAT).all(|r| &buf[r * 564..(r + 1) * 564] == &GOLDEN[..]);
    if bytes_ok {
        let enc = if aes.is_some() { " (AES-128)" } else { "" };
        println!("PASS: {tag} (received {got} bytes = GOLDEN x {REPEAT} byte-exact{enc})");
        exit(0);
    } else {
        eprintln!("FAIL[{tag}]: len_ok={len_ok} got={got}/{STREAM_LEN} bytes_ok={bytes_ok}");
        exit(1);
    }
}
