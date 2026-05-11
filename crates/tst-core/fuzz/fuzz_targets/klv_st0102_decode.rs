//! Fuzz target — decode panic-freedom + decode → encode → decode
//! round-trip for `klv::st0102::decode`.
//!
//! `decode` may return `Err` on any input, but must not panic. For
//! inputs that decode successfully, `encode_to_vec` followed by a
//! second `decode` must yield an equal `SecurityLs`. Catches
//! encode/decode asymmetry in typed fields — the failure mode that
//! pure panic-only fuzz misses.

#![no_main]
use libfuzzer_sys::fuzz_target;
use tst_core::klv::st0102::{decode, decode_strict, encode_to_vec};

fuzz_target!(|data: &[u8]| {
    // Lenient + strict decoders: panic-freedom probe (preserves
    // pre-Phase-6 coverage on decode_strict).
    let _ = decode_strict(data);
    let Ok(ls1) = decode(data) else { return; };

    // Round-trip is only well-defined when no `unknown` field has a
    // BER-OID multi-byte tag (> 127). The ST 0102 encoder intentionally
    // drops multi-byte unknown tags (klv/st0102/mod.rs §481-493: forward-
    // compat decoder accepts what the encoder cannot emit). Skip those
    // inputs; they're out of round-trip scope.
    if ls1.unknown.iter().any(|u| u.tag > 127) {
        return;
    }

    // decode → encode → decode must yield an equal SecurityLs.
    let bytes = match encode_to_vec(&ls1) {
        Ok(b) => b,
        Err(_) => return,  // encode-after-decode failure is a different
                           // bug class; out of scope for this round-trip.
    };
    let ls2 = decode(&bytes).expect("decode-after-encode must succeed");
    assert_eq!(ls1, ls2, "ST 0102 round-trip mismatch");
});
