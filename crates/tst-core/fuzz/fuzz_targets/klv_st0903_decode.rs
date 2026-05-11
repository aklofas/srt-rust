//! Fuzz target — decode panic-freedom + decode → encode → decode
//! round-trip for `klv::st0903::decode`.
//!
//! `decode` may return `Err` on any input, but must not panic. For
//! inputs that decode successfully, `encode_to_vec` followed by a
//! second `decode` must yield an equal `VmtiLs`. Catches encode/decode
//! asymmetry in typed fields — the failure mode that pure panic-only
//! fuzz misses.

#![no_main]
use libfuzzer_sys::fuzz_target;
use tst_core::klv::st0903::{decode, decode_strict, encode_to_vec};

fuzz_target!(|data: &[u8]| {
    // Lenient + strict decoders: panic-freedom probe (preserves
    // pre-Phase-6 coverage on decode_strict).
    let _ = decode_strict(data);
    let Ok(ls1) = decode(data) else { return; };

    // Round-trip: decode → encode → decode must yield an equal VmtiLs.
    // `field_errors` is excluded from PartialEq (manual impl) since
    // it's a decoder-side diagnostic, not part of the LS value.
    let bytes = match encode_to_vec(&ls1) {
        Ok(b) => b,
        Err(_) => return,
    };
    let ls2 = decode(&bytes).expect("decode-after-encode must succeed");
    assert_eq!(ls1, ls2, "ST 0903 round-trip mismatch");
});
