//! Decode ST 0102 Security Metadata Local Set from a .ts file.
//!
//! Walks the file's MPEG-TS packets, demuxes the elementary streams,
//! decodes each ST 0601 UAS Datalink LS record, then if Tag 48
//! (`security_local_set`) is present, decodes the inner ST 0102 LS via
//! the typed sibling-layer parser at [`tst_core::klv::st0102`].
//!
//! Why a separate example rather than an extension of
//! `klv_decode_file.rs`: ST 0102 is a *nested* LS (inside ST 0601
//! Tag 48). The sibling-layer composition pattern — call
//! `klv::st0102::decode` on `record.security_local_set.as_deref()` —
//! is the load-bearing teaching point. See `docs/guides/klv.md`
//! "Typed Security Local Set" section for the rationale.
//!
//! Most gimbaled-platform captures don't include classification
//! metadata, so this example will print "no security LS" on most
//! files. That's expected — ST 0102 is conditionally emitted only
//! when content is classified.
//!
//! Usage:
//!   cargo run -p tst-examples --example decode_security_metadata -- path/to/file.ts

use std::env;
use std::fs;

use tst_core::klv::st0102::{self, SecurityLs};
use tst_core::klv::st0601;
use tst_core::mpegts::demux::{DemuxEvent, Demuxer, MetadataKind};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = env::args()
        .nth(1)
        .ok_or_else(|| "usage: decode_security_metadata <path.ts>".to_string())?;

    // Slurp the whole file. The demuxer is fine with chunked feed
    // (see `extract_video_au.rs` for the streaming pattern), but a
    // single-shot read keeps this example focused on the
    // sibling-layer composition we're trying to teach.
    let bytes = fs::read(&path)?;
    eprintln!("loaded {} bytes from {path}", bytes.len());

    // Lenient-default Demuxer. Strict mode would convert
    // NonConformant events into errors via DemuxError::StrictRejection;
    // for an exploratory example we want to see whatever the file
    // contains, not bail at the first oddity.
    let mut d = Demuxer::new();
    d.feed(&bytes)
        .expect("demuxer recovered from any non-conformance");
    // `flush` empties any in-flight PES reassembly buffers — without
    // it, the trailing KLV record (or video AU) at end-of-file would
    // be silently dropped. Always call `flush` after the last `feed`.
    d.flush();

    let mut total_st0601 = 0usize;
    let mut with_security_ls = 0usize;
    let mut decoded_security = 0usize;

    while let Some(event) = d.next_event() {
        // Match the KLV-bearing event shape. The demuxer delivers KLV
        // via the top-level DemuxEvent::Metadata variant (not nested
        // inside SamplePayload — KLV is a metadata stream, not a
        // sample stream). We match both sync (KlvSyncAuCell — H.222.0
        // §2.12.4.2 AU cell wrap; the demuxer has already peeled the
        // 5-byte AU cell header) and async (KlvAsync — bare KLV LS,
        // typically 1-10 Hz). Both surface the inner KLV LS bytes
        // ready for `klv::st0601::decode` in the `payload` field.
        let payload = match event {
            DemuxEvent::Metadata {
                kind: MetadataKind::KlvSyncAuCell { .. } | MetadataKind::KlvAsync,
                payload,
                ..
            } => payload,
            // Other event variants (Sample, ProgramMap, Discontinuity,
            // NonConformant, Metadata::Unknown) are out of scope
            // here — we only want ST 0601 records.
            _ => continue,
        };

        total_st0601 += 1;

        // Decode the parent ST 0601 record.
        //
        // We use lenient `decode` here. A production validator would
        // use `decode_strict_compliance` (full ST 0601.8 mandatory-
        // ordering rules) or `decode_strict` (family UL gate); see
        // `klv_decode_file.rs` for the strictness ladder. For an
        // exploratory probe, lenient maximizes the records we can
        // inspect.
        let parent = match st0601::decode(&payload) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("  ST 0601 decode error: {e}");
                continue;
            }
        };

        // Tag 48 → bytes; sibling-layer call to ST 0102.
        //
        // `security_local_set` is `Option<Vec<u8>>` — the parent
        // typed surface preserves the inner bytes verbatim rather
        // than coupling the two decoders. Consumers wanting typed
        // access call `klv::st0102::decode` themselves, which is
        // exactly what we do here.
        let security_bytes = match parent.security_local_set.as_deref() {
            Some(b) if !b.is_empty() => b,
            _ => continue,
        };
        with_security_ls += 1;

        // Lenient decode — tolerates missing tags, unknown enum
        // codepoints (decoded as `Unknown(u8)`), and malformed UTF-16
        // on Tag 13 (raw bytes preserved in the `unknown` vec).
        // Strict mode (`decode_strict`) would additionally enforce:
        //   - Required tags 1, 2, 3, 12, 13, 22 all present.
        //   - No unknown enum codepoints on Tags 1, 2, 12.
        //   - No `OmittedValueXX` reserved codepoints on Tags 2, 12.
        //   - No malformed UTF-16 on Tag 13.
        //   - No duplicate tags.
        // Use strict in compliance pipelines that need to assert
        // upstream emitters meet ST 0102.12 §6.7 minimums.
        match st0102::decode(security_bytes) {
            Ok(sec) => {
                decoded_security += 1;
                print_security_record(&sec);
            }
            Err(e) => eprintln!("  ST 0102 decode error: {e}"),
        }
    }

    eprintln!(
        "\nsummary: {total_st0601} ST 0601 records, {with_security_ls} with Tag 48, \
         {decoded_security} decoded"
    );

    Ok(())
}

/// Pretty-print the most consumer-relevant typed fields. For
/// full-record inspection use `{:#?}` formatting on the `SecurityLs`
/// struct directly.
fn print_security_record(sec: &SecurityLs) {
    println!(
        "  classification: {:?}, country={:?} ({:?}), object_codes={:?} ({:?}), version={:?}",
        sec.security_classification,
        sec.classifying_country,
        sec.classifying_country_coding_method,
        sec.object_country_codes,
        sec.object_country_coding_method,
        sec.version,
    );
    if let Some(reason) = sec.classification_reason.as_deref() {
        println!("    reason: {reason}");
    }
    if let Some(declass) = sec.declassification_date.as_deref() {
        println!("    declassification: {declass}");
    }
    if !sec.unknown.is_empty() {
        println!(
            "    unknown tags preserved: {} entries (forward-compat)",
            sec.unknown.len()
        );
    }
    if !sec.field_errors.is_empty() {
        println!(
            "    field-level decode failures: {} (lenient-mode: typed field is None)",
            sec.field_errors.len()
        );
    }
}
