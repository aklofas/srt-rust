# Recipe 28: Decode security metadata from an ST 0601 record

> **When to use this:** ST 0601 Tag 48 (Security Local Set) is populated and you need ST 0102 classification, country codes, and version info.

> **Related:**
> - [guides/klv.md](/docs/guides/klv.md) — sibling-layer composition (ST 0601 → ST 0102)
> - [Example: `decode_security_metadata`](/examples/klv-metadata/decode_security_metadata.rs)

Sibling-layer composition: decode the parent ST 0601 LS, then if
Tag 48 is non-empty, run `klv::st0102::decode` on the inner bytes.

```rust
use tst_core::klv::{st0102, st0601};

# fn process(record_bytes: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
let parent = st0601::decode(record_bytes)?;

if let Some(security_bytes) = parent.security_local_set.as_deref() {
    let security = st0102::decode(security_bytes)?;
    println!(
        "classification={:?} country={:?} version={:?}",
        security.security_classification,
        security.classifying_country,
        security.version,
    );
}
# Ok(())
# }
```

Use `st0102::decode_strict` instead when the consumer wants the spec's
required-tag set enforced (e.g. compliance pipelines for classified
delivery).

Construct + encode the symmetric path:

```rust
use tst_core::klv::st0102::{
    self, ClassifyingCountryCodingMethod, ObjectCountryCodingMethod,
    SecurityClassification, SecurityLs,
};

let security = SecurityLs {
    security_classification: Some(SecurityClassification::Confidential),
    classifying_country_coding_method: Some(
        ClassifyingCountryCodingMethod::Iso3166ThreeLetter,
    ),
    classifying_country: Some("//USA".to_string()),
    object_country_coding_method: Some(
        ObjectCountryCodingMethod::Iso3166ThreeLetter,
    ),
    object_country_codes: Some("USA".to_string()),
    version: Some(12),
    ..Default::default()
};
let bytes = st0102::encode_to_vec(&security)?;
// Stuff `bytes` into a UasDatalinkLs.security_local_set field, then
// st0601::encode_to_vec the parent record.
```

See `examples/klv-metadata/decode_security_metadata.rs` for the full file-walking
example.
