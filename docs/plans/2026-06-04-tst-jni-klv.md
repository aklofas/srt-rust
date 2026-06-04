# tst-jni KLV wave (`org.tstrans.klv`) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Each task = a fresh implementer subagent; the controller verifies every diff against `tst_core` + tst-py source (NOT against this plan's snippets — see "Grounding rule"), then a spec-review + quality-review subagent per task, and a whole-diff review before PR.

**Goal:** Add the full typed-KLV `org.tstrans.klv` surface (decode **and** encode of ST 0601 / 0102 / 0605 / 0903) to the JVM binding, mirroring tst-py's `tstrans.klv` module-for-module, with the same per-module cross-binding parity obligation the prior mpegts waves carried.

**Architecture:** A new Java package `org.tstrans.klv` (immutable record value-types + 3 ST 0102 enums + a `Klv` static facade) backed by a new Rust JNI submodule `bindings/jvm/src/klv/`. Decode marshals Rust→Java via public `Builder`s on the 4 large sets (sidesteps the 80-arg `new_object` descriptor foot-gun); encode reads Java records field-by-field via accessors, mirroring tst-py's `py_to_*` translators. Two new checked-exception families (`KlvDecodeException`, `KlvEncodeException`); field-level errors are non-fatal and surface on each decoded set's `fieldErrors()` list. KLV byte payloads cross as heap read-only `ByteBuffer` copies (JDK-17-safe per spec §5.4). Stateless — no native handles, nothing `AutoCloseable`.

**Tech Stack:** Rust (`jni` 0.21, edition 2024), `tst_core::klv::*`, Java 17 (records + sealed types, **no `switch`-on-sealed in committed code**), Gradle 9.5.1 + JUnit5.

---

## Grounding rule (carries the lesson from the keystone + demux-completion + mux waves)

Every prior wave found 3–4 cases where a plan snippet diverged from the real Rust/tst-py type. **The canonical surface shape is tst-py**, which is itself a faithful mirror of `tst_core`:

- **Surface shape (field names, optionality, byte-vs-typed, decode/encode entry points, error mapping):** `bindings/python/python/tstrans/klv.py` + `bindings/python/src/klv.rs`. Read these for every set.
- **Field types + enum codepoints + entry-point signatures (authoritative):** `crates/tst-core/src/klv/{st0601,st0102,st0605,st0903}/{model.rs,enums.rs,mod.rs}` + `crates/tst-core/src/klv/st0903/vtarget_pack/model.rs`.
- **Error variants:** `crates/tst-core/src/error.rs` (`KlvDecodeError`, `KlvEncodeError`, `KlvFieldError`).
- The controller MUST diff each implementer's Java types against the tst-py dataclass field-for-field and the enum codepoints against `enums.rs` value-for-value before approving. Enum codepoints are **non-contiguous** (e.g. `ObjectCountryCodingMethod` jumps 0x0F→0x40) — Java enums MUST carry an explicit `code()` int, never rely on ordinal.

## Verified facts (already confirmed from source; use these directly)

**Entry points (`tst_core::klv::*`):**
- ST 0605: `st0605::{decode, encode}`. `decode(&[u8]) -> Result<PrecisionTimeStampPack, KlvDecodeError>`; `encode(&PrecisionTimeStampPack) -> Vec<u8>` (infallible, always 26 bytes).
- ST 0102: `st0102::{decode, decode_strict}`; `st0102::encode_to_vec(&SecurityLs) -> Result<Vec<u8>, KlvEncodeError>`. Decode takes **body-only** bytes (no UL/outer-BER).
- ST 0903: `st0903::{decode, decode_strict}`; `encode_to_vec(&VmtiLs)` (embedded body, no UL, no Tag-1 checksum) + `encode_to_vec_standalone(&VmtiLs)` (full `[UL][BER][body][Tag1 checksum]`). Decode takes **body-only**.
- ST 0601: `st0601::{decode, decode_strict, decode_strict_compliance}`; `encode_to_vec(&UasDatalinkLs)` + `encode_strict_compliance(&UasDatalinkLs)`. Decode takes the **full buffer including the 16-byte UL**.

**`KlvDecodeError → KlvDecodeException.Kind`** (port `klv_decode_error_to_pyerr` in `bindings/python/src/klv.rs` exactly):
| Rust variant(s) | Kind |
|---|---|
| `Truncated` / `MalformedLength` / `LengthOverflow` | `TRUNCATED_SET` |
| `UnexpectedUniversalLabel` | `BAD_UNIVERSAL_LABEL` |
| `ChecksumMismatch` | `CHECKSUM_MISMATCH` |
| `DuplicateTag` | `DUPLICATE_TAG` |
| `Tag2NotFirst` / `Tag1NotLast` / `MissingTag65` / `St0102MissingRequiredTag` / `St0903MissingRequiredTag` | `MISSING_REQUIRED_TAG` |
| `MalformedTag` / `NonCanonicalLength` / `NonCanonicalTag` / `TrailingBytes` / `BadTimeStampPackLength` / `ReservedBitsInvalid` / `St0903InvalidVTargetPack` / `FieldError(_)` | `MALFORMED_BYTES` |
| `_` (non_exhaustive) | `INTERNAL` |

→ `KlvDecodeException.Kind = { TRUNCATED_SET, BAD_UNIVERSAL_LABEL, CHECKSUM_MISMATCH, DUPLICATE_TAG, MISSING_REQUIRED_TAG, MALFORMED_BYTES, INTERNAL }` (7).

**`KlvEncodeError → KlvEncodeException.Kind`** (port `klv_encode_error_to_pyerr`):
`BufferTooSmall→BUFFER_TOO_SMALL`, `RecordTooLarge→RECORD_TOO_LARGE`, `OutOfRange→OUT_OF_RANGE`, `StringTooLong→STRING_TOO_LONG`, `UnsupportedImapbLength→UNSUPPORTED_IMAPB_LENGTH`, `InvalidImapbParams→INVALID_IMAPB_PARAMS`, `MissingMandatoryItem→MISSING_MANDATORY_ITEM`, `ReservedTagInUnknown→RESERVED_TAG_IN_UNKNOWN`, `_→BUFFER_TOO_SMALL`.
→ `KlvEncodeException.Kind = { BUFFER_TOO_SMALL, RECORD_TOO_LARGE, OUT_OF_RANGE, STRING_TOO_LONG, UNSUPPORTED_IMAPB_LENGTH, INVALID_IMAPB_PARAMS, MISSING_MANDATORY_ITEM, RESERVED_TAG_IN_UNKNOWN }` (8). (`KlvEncodeException` additionally carries an optional `Long tag` for the tag-bearing variants — mirror tst-py's `KlvEncodeError.tag`.)

**`KlvFieldErrorKind`** (9, non-fatal; from `bindings/python/python/tstrans/klv.py`): `OUT_OF_RANGE, INVALID_UTF8, INVALID_UTF16, INVALID_LENGTH, INVALID_SENTINEL, INVALID_CODEPOINT, TRUNCATED_FIELD, UNSUPPORTED_IMAPB_LENGTH, INVALID_IMAPB_PARAMS`. Mapping from `RustKlvFieldError` is in `convert_field_error` (port exactly; wildcard → `INVALID_LENGTH`).

**Well-known 16-byte ULs** (hardcode in Java exactly as tst-py's `klv.py` constants):
- `ST_0601_UL = 060e2b34020b01010e01030101000000`
- `SECURITY_LS_UL = 060e2b34020301010e01030302000000`
- `PRECISION_TIMESTAMP_PACK_UL = 060e2b34020501010e01010311000000`
- `VMTI_LS_UL = 060e2b34020b01010e01030306000000`
- `isSt0601Family(buf)`: `len>=16 && buf[0..13]==060e2b34020b01010e01030101 && buf[15]==0x00` (port `is_st0601_family`).

**Existing conventions to follow:**
- `bindings/jvm/src/error.rs` — `throw_kinded(env, exc_class, kind_sig, kind, message)` shared builder; `throw_demux`/`throw_mux` wrap it. The jvm error-mapping ratchet (`scripts/check/jvm/error-mapping-coverage.sh`) greps, per `Kind` constant, for `<makefn>(env, "<CONST>", ...)` with a **literal** 2nd arg — so all kind literals must appear inline (NOT a map-to-variable-then-call).
- `scripts/ratchets/error-mapping.tsv` — 5 tab-separated columns; add the two java rows with a trailing `-`.
- JNI handle/marshalling reference: `bindings/jvm/src/mpegts/mod.rs` + `bindings/jvm/src/mpegts/muxer.rs`. Heap-`ByteBuffer` helper: reuse the keystone's `wrap_heap_byte_buffer` (extract it to a shared `bindings/jvm/src/jutil.rs` in Task 0 if it's private to `mpegts`).
- `tests/coverage/surface-manifest.toml` already has rust rows for `tst_core::klv::st0601::{decode,encode}`; add `java:` columns / new rows in Task 5 (mirror the demux wave's `java:` graduation; resolution = last-dotted-leaf grep over `bindings/jvm/src/main/java`).

## File structure

**Java (`bindings/jvm/src/main/java/org/tstrans/`):**
- `KlvDecodeException.java`, `KlvEncodeException.java` (extend `BindingException`) — Task 0
- `klv/KlvFieldErrorKind.java`, `klv/KlvFieldError.java`, `klv/KlvUnknownField.java`, `klv/KlvSet.java` (sealed marker), `klv/Klv.java` (facade) — Task 0 (skeleton), filled across tasks
- `klv/TimeStatus.java`, `klv/PrecisionTimeStampPack.java` — Task 1
- `klv/SecurityClassification.java`, `klv/ClassifyingCountryCodingMethod.java`, `klv/ObjectCountryCodingMethod.java`, `klv/SecurityLs.java` (+ `Builder`) — Task 2
- `klv/VTargetPack.java` (+ `Builder`), `klv/VmtiLs.java` (+ `Builder`) — Task 3
- `klv/GeoPoint.java`, `klv/Attitude.java`, `klv/FieldOfView.java`, `klv/Corners.java`, `klv/UasDatalinkLs.java` (+ `Builder` + 6 composite accessors) — Task 4
- `module-info.java` — add `exports org.tstrans.klv;` (Task 0)

**Rust (`bindings/jvm/src/`):**
- `error.rs` — add `throw_klv_decode(&mut JNIEnv, &KlvDecodeError)` + `throw_klv_encode(&mut JNIEnv, &KlvEncodeError)` mappers (all literals inline) — Task 0
- `jutil.rs` (new, if extraction needed) — shared `wrap_heap_byte_buffer`, field-error-list + unknown-list build/read helpers — Task 0
- `klv/mod.rs` (new; `pub mod klv;` in `lib.rs`) — `pub mod st0605; pub mod st0102; pub mod st0903; pub mod st0601;` + a `_raise_klv_*_for_test` forced-throw fn — Task 0
- `klv/st0605.rs`, `klv/st0102.rs`, `klv/st0903.rs`, `klv/st0601.rs` — the per-set decode/encode JNI fns — Tasks 1–4

**Tests (`bindings/jvm/src/test/java/org/tstrans/`):**
- `KlvErrorModelTest.java` — Task 0
- `klv/St0605Test.java` — Task 1; `klv/St0102Test.java` — Task 2; `klv/St0903Test.java` — Task 3; `klv/St0601Test.java` — Task 4
- `klv/ParseUniversalTest.java` + extend `scenarios/ScenarioReproductionTest.java` — Task 5

**Marshalling pattern (applies to Tasks 2–4):** Each large set has a public mutable `Builder` with primitive/`String`/`ByteBuffer`/`Long`/`Double` setters (fluent, return `Builder`) + `build()`. **Decode (Rust→Java):** JNI calls `new <Set>$Builder()`, then `call_method` for each *present* field only (skip `None`/absent), then `build()`. **Encode (Java→Rust):** JNI reads each field via the record accessor (`call_method` returning the boxed/primitive value; null → `None`), builds the Rust struct, calls the encoder. The `unknown` list round-trips; entries whose tag collides with a typed tag are dropped (typed wins) — port the `is_st*_typed_tag` predicates from `bindings/python/src/klv.rs`.

---

## Task 0: Foundation — exception families, field-error model, facade skeleton, JNI scaffolding

**Files:**
- Create: `bindings/jvm/src/main/java/org/tstrans/KlvDecodeException.java`, `KlvEncodeException.java`
- Create: `bindings/jvm/src/main/java/org/tstrans/klv/{KlvFieldErrorKind,KlvFieldError,KlvUnknownField,KlvSet,Klv}.java`
- Create: `bindings/jvm/src/klv/mod.rs`, `bindings/jvm/src/jutil.rs`
- Modify: `bindings/jvm/src/lib.rs` (add `mod jutil; mod klv;`), `bindings/jvm/src/error.rs`, `bindings/jvm/src/main/java/module-info.java`, `scripts/ratchets/error-mapping.tsv`
- Test: `bindings/jvm/src/test/java/org/tstrans/KlvErrorModelTest.java`

- [ ] **Step 1: Write the two exception classes.** Mirror `DemuxException.java` exactly (constructor `(Kind, String)`, `serialVersionUID = 1L`, `kind()` accessor, `extends BindingException`). `KlvDecodeException.Kind` = the 7 constants above. `KlvEncodeException` adds an optional tag: a second constructor `(Kind, Long tag, String message)` + `Optional<Long> tag()`; the `(Kind, String)` ctor sets tag = null.

```java
// KlvDecodeException.java
package org.tstrans;

/** Thrown when typed-KLV decode rejects input. {@link Kind} mirrors the
 *  decode-error classification in {@code tst_core::error::KlvDecodeError}. */
public final class KlvDecodeException extends BindingException {
    private static final long serialVersionUID = 1L;

    public enum Kind {
        TRUNCATED_SET, BAD_UNIVERSAL_LABEL, CHECKSUM_MISMATCH,
        DUPLICATE_TAG, MISSING_REQUIRED_TAG, MALFORMED_BYTES, INTERNAL
    }

    private final Kind kind;

    public KlvDecodeException(Kind kind, String message) {
        super(message);
        this.kind = kind;
    }

    public Kind kind() {
        return kind;
    }
}
```

```java
// KlvEncodeException.java
package org.tstrans;

import java.util.Optional;

/** Thrown when typed-KLV encode fails. {@link Kind} mirrors
 *  {@code tst_core::error::KlvEncodeError}; {@link #tag()} carries the
 *  offending KLV tag for the tag-bearing variants. */
public final class KlvEncodeException extends BindingException {
    private static final long serialVersionUID = 1L;

    public enum Kind {
        BUFFER_TOO_SMALL, RECORD_TOO_LARGE, OUT_OF_RANGE, STRING_TOO_LONG,
        UNSUPPORTED_IMAPB_LENGTH, INVALID_IMAPB_PARAMS,
        MISSING_MANDATORY_ITEM, RESERVED_TAG_IN_UNKNOWN
    }

    private final Kind kind;
    private final Long tag; // nullable

    public KlvEncodeException(Kind kind, String message) {
        this(kind, null, message);
    }

    public KlvEncodeException(Kind kind, Long tag, String message) {
        super(message);
        this.kind = kind;
        this.tag = tag;
    }

    public Kind kind() {
        return kind;
    }

    public Optional<Long> tag() {
        return Optional.ofNullable(tag);
    }
}
```

- [ ] **Step 2: Write the field-error + unknown-field + sealed-set value types.** `KlvFieldErrorKind` = the 9 constants. `KlvFieldError` = `record KlvFieldError(KlvFieldErrorKind kind, long tag, String message)`. `KlvUnknownField` = `record KlvUnknownField(long tag, java.nio.ByteBuffer value)`. `KlvSet` = `public sealed interface KlvSet permits UasDatalinkLs, SecurityLs, PrecisionTimeStampPack, VmtiLs {}` (the 4 typed sets implement it; written here, set types added in later tasks — the `permits` clause references types not yet created, so this file compiles only once Task 1–4 stub those types; to keep Task 0 self-compiling, declare `KlvSet` with an **empty `permits`-less** sealed-by-package form is NOT possible — instead make `KlvSet` a plain marker interface in Task 0 and convert it to `sealed ... permits ...` in Task 5 once all 4 exist). **Decision: ship `KlvSet` as a plain (non-sealed) marker interface in Task 0; seal it in Task 5.**

```java
// klv/KlvSet.java  (Task 0 form: plain marker; sealed in Task 5)
package org.tstrans.klv;

/** Marker for the four typed KLV sets returned by {@link Klv#parseUniversal}.
 *  Sealed in the dispatcher task once all permitted types exist. */
public interface KlvSet {}
```

- [ ] **Step 3: Write the `Klv` facade skeleton** — UL constants as read-only `ByteBuffer`s + `isSt0601Family(byte[])`. Decode/encode/parseUniversal methods are added in later tasks. Use a private `ul(String hex)` helper returning `ByteBuffer.wrap(HexFormat.of().parseHex(hex)).asReadOnlyBuffer()`.

```java
// klv/Klv.java  (skeleton)
package org.tstrans.klv;

import java.nio.ByteBuffer;
import java.util.HexFormat;

/** Static facade for MISB typed-KLV decode/encode (ST 0601/0102/0605/0903).
 *  Mirrors tst-py's {@code tstrans.klv} free functions. */
public final class Klv {
    private Klv() {}

    public static final byte[] ST_0601_UL = HexFormat.of().parseHex("060e2b34020b01010e01030101000000");
    public static final byte[] SECURITY_LS_UL = HexFormat.of().parseHex("060e2b34020301010e01030302000000");
    public static final byte[] PRECISION_TIMESTAMP_PACK_UL = HexFormat.of().parseHex("060e2b34020501010e01010311000000");
    public static final byte[] VMTI_LS_UL = HexFormat.of().parseHex("060e2b34020b01010e01030306000000");

    /** Mirror of Rust {@code UniversalLabel::is_st0601_family}. */
    public static boolean isSt0601Family(byte[] buf) {
        if (buf.length < 16) return false;
        byte[] canonical = HexFormat.of().parseHex("060e2b34020b01010e01030101");
        for (int i = 0; i < 13; i++) if (buf[i] != canonical[i]) return false;
        return buf[15] == 0x00;
    }

    static { NativeLoader.ensureLoaded(); } // load libtstjni for the native decode/encode fns
}
```
(If `NativeLoader.ensureLoaded()` isn't the existing entry point, match whatever `Demuxer`/`Muxer` use to trigger native loading — check `NativeLoader.java`.)

- [ ] **Step 4: Write the two throw mappers in `error.rs`** with ALL kind literals inline (satisfies the ratchet immediately, independent of the decode/encode entry points). Add `throw_klv_decode(env, &KlvDecodeError)` and `throw_klv_encode(env, &KlvEncodeError)`. Each does an inline `match` calling a thin `throw_klv_decode_kind(env, "<CONST>", msg)` / `throw_klv_encode_kind(env, "<CONST>", tag, msg)` — **but the ratchet greps for `throw_klv_decode(env, "<CONST>", ...)`**, so name the literal-call helper exactly `throw_klv_decode` and `throw_klv_encode` and have the mapper be a *separate* fn. Cleanest: name the public mappers `map_klv_decode_error` / `map_klv_encode_error`, and the literal-throwing primitives `throw_klv_decode(env, kind, msg)` / `throw_klv_encode(env, kind, tag, msg)` (these are what the TSV `makefn` column names). The mapper calls the primitive once per arm with a literal kind.

```rust
// in error.rs
use tst_core::error::{KlvDecodeError, KlvEncodeError};

/// Throw `org.tstrans.KlvDecodeException(Kind.<kind>, message)`.
pub fn throw_klv_decode(env: &mut JNIEnv, kind: &str, message: &str) {
    if env.exception_check().unwrap_or(false) { return; }
    if let Err(e) = throw_kinded(
        env, "org/tstrans/KlvDecodeException",
        "Lorg/tstrans/KlvDecodeException$Kind;", kind, message,
    ) {
        let _ = env.throw_new("java/lang/RuntimeException",
            format!("KlvDecodeException throw failed ({kind}): {e}"));
    }
}

/// Throw `org.tstrans.KlvEncodeException(Kind.<kind>, tag, message)`.
/// `tag` = None → the (Kind, String) ctor; Some(t) → the (Kind, Long, String) ctor.
pub fn throw_klv_encode(env: &mut JNIEnv, kind: &str, tag: Option<u32>, message: &str) {
    if env.exception_check().unwrap_or(false) { return; }
    if let Err(e) = throw_klv_encode_inner(env, kind, tag, message) {
        let _ = env.throw_new("java/lang/RuntimeException",
            format!("KlvEncodeException throw failed ({kind}): {e}"));
    }
}

fn throw_klv_encode_inner(env: &mut JNIEnv, kind: &str, tag: Option<u32>, message: &str)
    -> jni::errors::Result<()> {
    let kind_sig = "Lorg/tstrans/KlvEncodeException$Kind;";
    let kind_val = env.get_static_field("org/tstrans/KlvEncodeException$Kind", kind, kind_sig)?.l()?;
    let msg = env.new_string(message)?;
    let exc = match tag {
        Some(t) => {
            let boxed = env.new_object("java/lang/Long", "(J)V",
                &[JValue::Long(i64::from(t))])?;
            env.new_object("org/tstrans/KlvEncodeException",
                &format!("({kind_sig}Ljava/lang/Long;Ljava/lang/String;)V"),
                &[JValue::Object(&kind_val), JValue::Object(&boxed), JValue::Object(&msg)])?
        }
        None => env.new_object("org/tstrans/KlvEncodeException",
            &format!("({kind_sig}Ljava/lang/String;)V"),
            &[JValue::Object(&kind_val), JValue::Object(&msg)])?,
    };
    env.throw(jni::objects::JThrowable::from(exc))
}

/// Map + throw a Rust `KlvDecodeError`. Inline literals = the 7 Kinds.
pub fn map_klv_decode_error(env: &mut JNIEnv, e: &KlvDecodeError) {
    let msg = e.to_string();
    match e {
        KlvDecodeError::Truncated { .. } | KlvDecodeError::MalformedLength { .. }
        | KlvDecodeError::LengthOverflow { .. } => throw_klv_decode(env, "TRUNCATED_SET", &msg),
        KlvDecodeError::UnexpectedUniversalLabel { .. } => throw_klv_decode(env, "BAD_UNIVERSAL_LABEL", &msg),
        KlvDecodeError::ChecksumMismatch { .. } => throw_klv_decode(env, "CHECKSUM_MISMATCH", &msg),
        KlvDecodeError::DuplicateTag { .. } => throw_klv_decode(env, "DUPLICATE_TAG", &msg),
        KlvDecodeError::Tag2NotFirst | KlvDecodeError::Tag1NotLast | KlvDecodeError::MissingTag65
        | KlvDecodeError::St0102MissingRequiredTag { .. }
        | KlvDecodeError::St0903MissingRequiredTag { .. } => throw_klv_decode(env, "MISSING_REQUIRED_TAG", &msg),
        KlvDecodeError::MalformedTag { .. } | KlvDecodeError::NonCanonicalLength { .. }
        | KlvDecodeError::NonCanonicalTag { .. } | KlvDecodeError::TrailingBytes { .. }
        | KlvDecodeError::BadTimeStampPackLength { .. } | KlvDecodeError::ReservedBitsInvalid { .. }
        | KlvDecodeError::St0903InvalidVTargetPack { .. } | KlvDecodeError::FieldError(_) =>
            throw_klv_decode(env, "MALFORMED_BYTES", &msg),
        _ => throw_klv_decode(env, "INTERNAL", &msg),
    }
}

/// Map + throw a Rust `KlvEncodeError`. Inline literals = the 8 Kinds.
pub fn map_klv_encode_error(env: &mut JNIEnv, e: &KlvEncodeError) {
    let msg = e.to_string();
    use KlvEncodeError as E;
    match e {
        E::BufferTooSmall { .. } => throw_klv_encode(env, "BUFFER_TOO_SMALL", None, &msg),
        E::RecordTooLarge => throw_klv_encode(env, "RECORD_TOO_LARGE", None, &msg),
        E::OutOfRange { tag, .. } => throw_klv_encode(env, "OUT_OF_RANGE", Some(*tag), &msg),
        E::StringTooLong { tag, .. } => throw_klv_encode(env, "STRING_TOO_LONG", Some(*tag), &msg),
        E::UnsupportedImapbLength { .. } => throw_klv_encode(env, "UNSUPPORTED_IMAPB_LENGTH", None, &msg),
        E::InvalidImapbParams { .. } => throw_klv_encode(env, "INVALID_IMAPB_PARAMS", None, &msg),
        E::MissingMandatoryItem { tag, .. } => throw_klv_encode(env, "MISSING_MANDATORY_ITEM", Some(u32::from(*tag)), &msg),
        E::ReservedTagInUnknown { tag } => throw_klv_encode(env, "RESERVED_TAG_IN_UNKNOWN", Some(*tag), &msg),
        _ => throw_klv_encode(env, "BUFFER_TOO_SMALL", None, &msg),
    }
}
```
(Verify the exact `KlvEncodeError` variant field shapes against `crates/tst-core/src/error.rs` — `MissingMandatoryItem.tag` is a `u8` per tst-py's `u32::from(*tag)`.)

- [ ] **Step 5: Write `jutil.rs` shared helpers.** Extract/define: `wrap_heap_byte_buffer(env, &[u8]) -> JObject` (a `ByteBuffer.wrap(byte[])` over a fresh copy, read-only — copy the keystone's impl from `mpegts/mod.rs`; if it's private there, move it here and re-point `mpegts`). Define `build_field_errors(env, &[KlvFieldError]) -> JObject` (returns a `java.util.List<KlvFieldError>` built via `ArrayList`; map each via `convert_field_error` logic → `new KlvFieldError(Kind, long, String)`), and `build_unknown_list(env, &[OwnedRawField]) -> JObject` (`List<KlvUnknownField>`), and `read_unknown_list(env, &JObject, is_typed: impl Fn(u32)->bool) -> Vec<OwnedRawField>` (iterate the Java `List<KlvUnknownField>`, read `.tag()`/`.value()`, drop typed-tag collisions). Keep the `KlvFieldErrorKind` name table here.

- [ ] **Step 6: Write `klv/mod.rs`** declaring `pub mod st0605; pub mod st0102; pub mod st0903; pub mod st0601;` (empty modules for now — each gets a `// filled in Task N` note) + one test-only forced-throw fn per family so the JVM `KlvErrorModelTest` can exercise the wiring before real entry points exist:

```rust
// klv/mod.rs
pub mod st0102;
pub mod st0601;
pub mod st0605;
pub mod st0903;

use jni::JNIEnv;
use jni::objects::{JClass, JString};

use crate::error::{throw_klv_decode, throw_klv_encode};

/// Test-only: `org.tstrans.klv.Klv.nRaiseDecodeForTest(kind)`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_klv_Klv_nRaiseDecodeForTest<'l>(
    mut env: JNIEnv<'l>, _c: JClass<'l>, kind: JString<'l>,
) {
    let k: String = env.get_string(&kind).map(Into::into).unwrap_or_default();
    throw_klv_decode(&mut env, &k, "forced decode error for test");
}

/// Test-only: `org.tstrans.klv.Klv.nRaiseEncodeForTest(kind)`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_klv_Klv_nRaiseEncodeForTest<'l>(
    mut env: JNIEnv<'l>, _c: JClass<'l>, kind: JString<'l>,
) {
    let k: String = env.get_string(&kind).map(Into::into).unwrap_or_default();
    throw_klv_encode(&mut env, &k, None, "forced encode error for test");
}
```
Add the matching `private static native void nRaiseDecodeForTest(String)` / `nRaiseEncodeForTest(String)` + package-private `static void raiseDecodeForTest(String)`/`raiseEncodeForTest(String)` wrappers on `Klv.java`.

- [ ] **Step 7: Wire `lib.rs` + `module-info.java` + the TSV.** `lib.rs`: add `mod jutil;` and `mod klv;`. `module-info.java`: add `exports org.tstrans.klv;`. `scripts/ratchets/error-mapping.tsv`: append two rows (REAL tabs):
```
java	klv_decode	KlvDecodeException.Kind	throw_klv_decode	-
java	klv_encode	KlvEncodeException.Kind	throw_klv_encode	-
```

- [ ] **Step 8: Write `KlvErrorModelTest.java`** — for each `KlvDecodeException.Kind` and `KlvEncodeException.Kind` constant, call `Klv.raiseDecodeForTest(name)` / `Klv.raiseEncodeForTest(name)`, assert the thrown exception is the right class with `kind()` equal to that constant. (Mirror `ErrorModelTest.java` / `MuxErrorModelTest.java`.)

```java
package org.tstrans;

import static org.junit.jupiter.api.Assertions.*;
import org.junit.jupiter.api.Test;
import org.tstrans.klv.Klv;

class KlvErrorModelTest {
    @Test void decodeKindsRoundTrip() {
        for (KlvDecodeException.Kind k : KlvDecodeException.Kind.values()) {
            KlvDecodeException ex = assertThrows(KlvDecodeException.class,
                () -> Klv.raiseDecodeForTest(k.name()));
            assertEquals(k, ex.kind());
        }
    }
    @Test void encodeKindsRoundTrip() {
        for (KlvEncodeException.Kind k : KlvEncodeException.Kind.values()) {
            KlvEncodeException ex = assertThrows(KlvEncodeException.class,
                () -> Klv.raiseEncodeForTest(k.name()));
            assertEquals(k, ex.kind());
        }
    }
}
```

- [ ] **Step 9: Build + test + ratchet.**
Run: `SRT_FORCE_VENDORED=1 cargo build -p tst-jni && cargo fmt --all -- --check && SRT_FORCE_VENDORED=1 cargo clippy -p tst-jni --all-targets -- -D warnings`
Run: `cd bindings/jvm && ./gradlew test --no-daemon` — Expected: PASS (KlvErrorModelTest green).
Run: `bash scripts/check/jvm/error-mapping-coverage.sh </dev/null` — Expected: `jvm error-mapping coverage: OK`.

- [ ] **Step 10: Commit.**
```bash
git add bindings/jvm scripts/ratchets/error-mapping.tsv
git commit -m "tst-jni klv: foundation — exception families, field-error model, JNI scaffolding"
```

---

## Task 1: ST 0605 — Precision Time Stamp Pack (the vertical-slice pattern proof)

**Source of truth:** `tstrans.klv.{TimeStatus,PrecisionTimeStampPack}` + `decode_precision_timestamp`/`encode_precision_timestamp` (`klv.py`/`klv.rs`); `crates/tst-core/src/klv/st0605/model.rs`.

**Files:** Create `klv/TimeStatus.java`, `klv/PrecisionTimeStampPack.java`, `bindings/jvm/src/klv/st0605.rs`; modify `klv/Klv.java`; Test `klv/St0605Test.java`.

- [ ] **Step 1: Write `TimeStatus.java`** — `record TimeStatus(int raw)` with a compact ctor validating `0 <= raw <= 0xFF` (throw `IllegalArgumentException`), and `isLocked()`/`hasDiscontinuity()`/`isReverseJump()`/`reservedBitsValid()` derived from the bitmasks in `st0605/model.rs` (0x80/0x40/0x20/0x1F).

- [ ] **Step 2: Write `PrecisionTimeStampPack.java`** — `record PrecisionTimeStampPack(TimeStatus timeStatus, long timestampUs) implements KlvSet {}`.

- [ ] **Step 3: Write `klv/St0605Test.java` (failing — types/methods missing).** Port `bindings/python/tests/test_klv_st0605.py` cases. Concrete cases:
```java
package org.tstrans.klv;
import static org.junit.jupiter.api.Assertions.*;
import org.junit.jupiter.api.Test;
import java.util.HexFormat;
import org.tstrans.KlvDecodeException;

class St0605Test {
    // 16-byte UL + BER 0x09 + status 0x1F + 8-byte BE microsecond ts
    private static byte[] pack(long us, int status) {
        byte[] out = new byte[26];
        System.arraycopy(Klv.PRECISION_TIMESTAMP_PACK_UL, 0, out, 0, 16);
        out[16] = 0x09; out[17] = (byte) status;
        for (int i = 0; i < 8; i++) out[25 - i] = (byte) (us >>> (8 * i));
        return out;
    }
    @Test void decodeLocked() {
        PrecisionTimeStampPack p = Klv.decodePrecisionTimestamp(pack(1_753_983_356_565_441L, 0x1F));
        assertTrue(p.timeStatus().isLocked());
        assertTrue(p.timeStatus().reservedBitsValid());
        assertEquals(1_753_983_356_565_441L, p.timestampUs());
    }
    @Test void decodeRejectsWrongUl() {
        byte[] b = pack(0, 0x1F);
        System.arraycopy(HexFormat.of().parseHex("060e2b34020b01010e01030101000000"), 0, b, 0, 16);
        KlvDecodeException ex = assertThrows(KlvDecodeException.class, () -> Klv.decodePrecisionTimestamp(b));
        assertEquals(KlvDecodeException.Kind.BAD_UNIVERSAL_LABEL, ex.kind());
    }
    @Test void encodeRoundTrip() {
        PrecisionTimeStampPack in = new PrecisionTimeStampPack(new TimeStatus(0x1F), 1_700_000_000_123_456L);
        byte[] wire = Klv.encodePrecisionTimestamp(in);
        assertEquals(26, wire.length);
        assertEquals(in, Klv.decodePrecisionTimestamp(wire));
    }
}
```

- [ ] **Step 4: Write `klv/st0605.rs`** — `nDecodePrecisionTimestamp(byte[]) -> jobject` and `nEncodePrecisionTimestamp(PrecisionTimeStampPack) -> byte[]`.

```rust
// klv/st0605.rs
use jni::JNIEnv;
use jni::objects::{JByteArray, JClass, JObject, JValue};
use jni::sys::jobject;
use tst_core::klv::st0605::{decode, encode, PrecisionTimeStampPack, TimeStatus};
use crate::error::map_klv_decode_error;

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_klv_Klv_nDecodePrecisionTimestamp<'l>(
    mut env: JNIEnv<'l>, _c: JClass<'l>, buf: JByteArray<'l>,
) -> jobject {
    let bytes = match env.convert_byte_array(&buf) { Ok(b) => b, Err(e) => {
        let _ = env.throw_new("java/lang/RuntimeException", format!("byte[] read: {e}"));
        return JObject::null().into_raw();
    }};
    match decode(&bytes) {
        Ok(p) => build_pack(&mut env, &p).unwrap_or_else(|_| JObject::null().into_raw()),
        Err(e) => { map_klv_decode_error(&mut env, &e); JObject::null().into_raw() }
    }
}

fn build_pack(env: &mut JNIEnv, p: &PrecisionTimeStampPack) -> jni::errors::Result<jobject> {
    let ts = env.new_object("org/tstrans/klv/TimeStatus", "(I)V",
        &[JValue::Int(i32::from(p.time_status.0))])?;
    let pack = env.new_object("org/tstrans/klv/PrecisionTimeStampPack",
        "(Lorg/tstrans/klv/TimeStatus;J)V",
        &[JValue::Object(&ts), JValue::Long(p.timestamp_us as i64)])?;
    Ok(pack.into_raw())
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_klv_Klv_nEncodePrecisionTimestamp<'l>(
    mut env: JNIEnv<'l>, _c: JClass<'l>, pack: JObject<'l>,
) -> jobject {
    // read TimeStatus.raw (int) + timestampUs (long) via accessors
    let ts = env.call_method(&pack, "timeStatus", "()Lorg/tstrans/klv/TimeStatus;", &[])
        .and_then(|v| v.l());
    let ts = match ts { Ok(o) => o, Err(e) => { let _ = env.throw_new("java/lang/RuntimeException", format!("{e}")); return JObject::null().into_raw(); }};
    let raw = env.call_method(&ts, "raw", "()I", &[]).and_then(|v| v.i()).unwrap_or(0) as u8;
    let us = env.call_method(&pack, "timestampUs", "()J", &[]).and_then(|v| v.j()).unwrap_or(0) as u64;
    let wire = encode(&PrecisionTimeStampPack { time_status: TimeStatus(raw), timestamp_us: us });
    env.byte_array_from_slice(&wire).map(JByteArray::into_raw)
        .unwrap_or_else(|_| JObject::null().into_raw())
}
```

- [ ] **Step 5: Add facade methods to `Klv.java`:**
```java
public static PrecisionTimeStampPack decodePrecisionTimestamp(byte[] buf) {
    return nDecodePrecisionTimestamp(buf);
}
public static byte[] encodePrecisionTimestamp(PrecisionTimeStampPack pack) {
    return nEncodePrecisionTimestamp(pack);
}
private static native PrecisionTimeStampPack nDecodePrecisionTimestamp(byte[] buf);
private static native byte[] nEncodePrecisionTimestamp(PrecisionTimeStampPack pack);
```
(`decodePrecisionTimestamp` declares `throws KlvDecodeException`? — `KlvDecodeException` is checked. Native methods can throw checked exceptions across JNI; declare `throws KlvDecodeException` on the public method and the native. Add it.)

- [ ] **Step 6: Build + test.**
Run: `SRT_FORCE_VENDORED=1 cargo build -p tst-jni && cargo fmt --all -- --check && SRT_FORCE_VENDORED=1 cargo clippy -p tst-jni --all-targets -- -D warnings`
Run: `cd bindings/jvm && ./gradlew test --no-daemon --tests 'org.tstrans.klv.St0605Test'` — Expected: PASS.

- [ ] **Step 7: Commit.** `git commit -am "tst-jni klv: ST 0605 Precision Time Stamp Pack decode+encode"`

---

## Task 2: ST 0102 — Security Metadata LS (3 enums + Builder marshalling pattern)

**Source of truth:** `tstrans.klv.{SecurityClassification,ClassifyingCountryCodingMethod,ObjectCountryCodingMethod,SecurityLs}` + `decode_security`/`encode_security` (`klv.py`/`klv.rs`); `crates/tst-core/src/klv/st0102/{enums.rs,model.rs}`; `is_st0102_typed_tag` (= `1..=14 | 22 | 23 | 24`).

**Files:** Create `klv/SecurityClassification.java`, `klv/ClassifyingCountryCodingMethod.java`, `klv/ObjectCountryCodingMethod.java`, `klv/SecurityLs.java`, `bindings/jvm/src/klv/st0102.rs`; modify `klv/Klv.java`; Test `klv/St0102Test.java`.

- [ ] **Step 1: Write the 3 enums** with explicit `code()` + `static Optional<E> fromCode(int)`. Codepoints are **non-contiguous** — copy them value-for-value from `st0102/enums.rs` (verified):
  - `SecurityClassification`: UNCLASSIFIED(1), RESTRICTED(2), CONFIDENTIAL(3), SECRET(4), TOP_SECRET(5).
  - `ClassifyingCountryCodingMethod`: the 16 values 0x01–0x10 (incl. `OMITTED_VALUE_08/09`) per `enums.rs`.
  - `ObjectCountryCodingMethod`: 0x01–0x0F + `GENC_ADMIN_SUB(0x40)` per `enums.rs` (note ISO_3166_NUMERIC=0x03, FIPS_104_TWO_LETTER=0x04 — different from Tag 2).
```java
package org.tstrans.klv;
import java.util.Optional;
public enum SecurityClassification {
    UNCLASSIFIED(0x01), RESTRICTED(0x02), CONFIDENTIAL(0x03), SECRET(0x04), TOP_SECRET(0x05);
    private final int code;
    SecurityClassification(int code) { this.code = code; }
    public int code() { return code; }
    public static Optional<SecurityClassification> fromCode(int c) {
        for (var v : values()) if (v.code == c) return Optional.of(v);
        return Optional.empty();
    }
}
```
(Write the other two identically with their codepoints.)

- [ ] **Step 2: Write `SecurityLs.java`** as a record with `implements KlvSet` + a public mutable `Builder`. **Forward-compat enum fields:** mirror tst-py's "enum or raw int" by storing the raw codepoint as a nullable `Integer` (null = tag absent) and exposing BOTH a raw accessor and a typed `Optional<E>` accessor. Fields (port the 17 typed + `unknown` + `fieldErrors` from `klv.py` `SecurityLs`; the 3 enum fields stored as `Integer ...Code`):
  - `Integer securityClassificationCode`, `Integer classifyingCountryCodingMethodCode`, `String classifyingCountry`, `Integer objectCountryCodingMethodCode`, `String objectCountryCodes`, `Integer version`, `String sciShiInfo`, `String caveats`, `String releasingInstructions`, `String classifiedBy`, `String derivedFrom`, `String classificationReason`, `String declassificationDate`, `String classificationMarkingSystem`, `String classificationComments`, `String classifyingCountryCodingMethodVersionDate`, `String objectCountryCodingMethodVersionDate`, `List<KlvUnknownField> unknown`, `List<KlvFieldError> fieldErrors`.
  - Typed accessors: `Optional<SecurityClassification> securityClassification()` = `securityClassificationCode == null ? empty : SecurityClassification.fromCode(code)`; same for the other two.
  - `Builder` has a setter per field (the 3 enum ones accept `int` and store the boxed `Integer`; provide overloads `securityClassification(SecurityClassification)` → stores `.code()`); `unknown(List)`/`fieldErrors(List)` default to empty list. `build()` validates non-null lists.

- [ ] **Step 3: Write `klv/St0102Test.java` (failing).** Port key cases from `bindings/python/tests/test_klv_st0102.py` + `test_klv_st0102_enums.py` + `test_klv_encode_st0102_st0605.py`. Concrete minimum:
```java
@Test void decodeMinimalBody() {
    // Tag 1 (Security Classification) = 0x01 (UNCLASSIFIED), 1-byte value
    SecurityLs s = Klv.decodeSecurity(new byte[]{0x01, 0x01, 0x01});
    assertEquals(Optional.of(SecurityClassification.UNCLASSIFIED), s.securityClassification());
}
@Test void encodeRoundTrip() {
    SecurityLs s = Klv.decodeSecurity(new byte[]{0x01, 0x01, 0x01});
    byte[] wire = Klv.encodeSecurity(s);
    assertEquals(Optional.of(SecurityClassification.UNCLASSIFIED), Klv.decodeSecurity(wire).securityClassification());
}
@Test void strictRejectsMissingRequiredTags() {
    // body with only Tag 1 → strict mode wants 1,2,3,12,13,22
    KlvDecodeException ex = assertThrows(KlvDecodeException.class,
        () -> Klv.decodeSecurity(new byte[]{0x01, 0x01, 0x01}, true));
    assertEquals(KlvDecodeException.Kind.MISSING_REQUIRED_TAG, ex.kind());
}
@Test void unknownCodepointSurfacesAsRawCode() {
    // Tag 1 with an out-of-spec codepoint 0xFE → typed accessor empty, raw code preserved
    SecurityLs s = Klv.decodeSecurity(new byte[]{0x01, 0x01, (byte)0xFE});
    assertTrue(s.securityClassification().isEmpty());
    assertEquals(254, (int) s.securityClassificationCode());
}
```
(Confirm the exact strict-required-tag fixture against `st0102/tests.rs` / tst-py; adjust the malformed body if `decode_strict` produces a different first failure.)

- [ ] **Step 4: Write `bindings/jvm/src/klv/st0102.rs`** — `nDecodeSecurity(byte[], boolean strict) -> jobject` (build via `SecurityLs$Builder`: `new_object` the builder, then per-present-field `call_method` setters, then `build()`); `nEncodeSecurity(SecurityLs) -> byte[]` (read fields via accessors → Rust `SecurityLs` → `encode_to_vec`, mapping `KlvEncodeError`). Port the field set + the enum `from_u8`/`to_u8` + `unknown` collision-drop from `bindings/python/src/klv.rs` (`convert_security_ls` / `py_to_security_ls`). Use `crate::jutil::{build_field_errors, build_unknown_list, read_unknown_list}`. Decode reads the 3 enum codepoints with `to_u8()`; build the Java `Integer` via boxing.

  Builder-call helper sketch (the pattern reused in Tasks 3–4):
```rust
// build a SecurityLs via its Builder, setting only present fields
fn build_security(env: &mut JNIEnv, s: &SecurityLs) -> jni::errors::Result<jobject> {
    let b = env.new_object("org/tstrans/klv/SecurityLs$Builder", "()V", &[])?;
    if let Some(v) = s.security_classification {
        env.call_method(&b, "securityClassification", "(I)Lorg/tstrans/klv/SecurityLs$Builder;",
            &[JValue::Int(i32::from(v.to_u8()))])?;
    }
    // ... one block per field (String via new_string + "(Ljava/lang/String;)L..Builder;") ...
    let fe = crate::jutil::build_field_errors(env, &s.field_errors)?;
    env.call_method(&b, "fieldErrors", "(Ljava/util/List;)Lorg/tstrans/klv/SecurityLs$Builder;",
        &[JValue::Object(&fe)])?;
    let unk = crate::jutil::build_unknown_list(env, &s.unknown)?;
    env.call_method(&b, "unknown", "(Ljava/util/List;)Lorg/tstrans/klv/SecurityLs$Builder;",
        &[JValue::Object(&unk)])?;
    let built = env.call_method(&b, "build", "()Lorg/tstrans/klv/SecurityLs;", &[])?.l()?;
    Ok(built.into_raw())
}
```

- [ ] **Step 5: Add facade methods to `Klv.java`:** `decodeSecurity(byte[])` (lenient), `decodeSecurity(byte[], boolean strict)`, `encodeSecurity(SecurityLs)` (+ the 3 native decls), all `throws KlvDecodeException`/`KlvEncodeException` as appropriate.

- [ ] **Step 6: Build + test.** Same cargo gate as Task 1; `./gradlew test --no-daemon --tests 'org.tstrans.klv.St0102Test'` — Expected: PASS.

- [ ] **Step 7: Commit.** `git commit -am "tst-jni klv: ST 0102 Security LS decode+encode (3 enums, Builder marshalling)"`

---

## Task 3: ST 0903 — VMTI LS + VTargetPack

**Source of truth:** `tstrans.klv.{VTargetPack,VmtiLs}` + `decode_vmti`/`encode_vmti`/`encode_vmti_standalone` (`klv.py`/`klv.rs`); `crates/tst-core/src/klv/st0903/{model.rs,vtarget_pack/model.rs}`; typed-tag predicates `is_st0903_vmti_typed_tag` (`1..=13 | 101..=103`), `is_st0903_vtarget_typed_tag` (`1..=23 | 100..=107`).

**Files:** Create `klv/VTargetPack.java` (+Builder), `klv/VmtiLs.java` (+Builder), `bindings/jvm/src/klv/st0903.rs`; modify `klv/Klv.java`; Test `klv/St0903Test.java`.

- [ ] **Step 1: Write `VTargetPack.java`** — record + Builder, fields ported field-for-field from `klv.py` `VTargetPack` (mandatory `long targetId`; the `Integer`/`Long`/`Double` optionals; `int[] targetColor` 3-element RGB validated in the compact ctor to length 3 & 0..255 — OR model as a small `record TargetColor(int r,int g,int b)` for value-equality; **use `TargetColor`** to keep record equality clean; nested-LS pass-through as `ByteBuffer`: `targetLocation, geospatialContourSeries, vmask, vtracker, vchip, vchipSeries, vobjectSeries`; `List<KlvUnknownField> unknown`; `List<KlvFieldError> fieldErrors`). Does NOT implement `KlvSet` (it's carried inside `VmtiLs.targets`).

- [ ] **Step 2: Write `VmtiLs.java`** — `record ... implements KlvSet` + Builder, fields from `klv.py` `VmtiLs` (`Integer checksum`, `Long precisionTimeStamp`, `String vmtiSystemName`, `Integer versionNumber`, `Long totalTargetsInFrame`, `Long numTargetsReported`, `Long frameWidth`, `Long frameHeight`, `String sourceSensor`, `Double horizontalFov`, `Double verticalFov`, `ByteBuffer miisId`, `List<VTargetPack> targets`, `ByteBuffer algorithmSeries`, `ByteBuffer ontologySeries`, `List<KlvUnknownField> unknown`, `List<KlvFieldError> fieldErrors`). Verify integer widths against `st0903/model.rs` (`frame_width` etc. are `u32` → Java `Long` to be safe; `version_number` `u16` → `Integer`).

- [ ] **Step 3: Write `klv/St0903Test.java` (failing).** Port from `bindings/python/tests/test_klv_st0903.py` + `test_klv_vtarget_pack.py` + `test_klv_encode_st0903.py`. Minimum: decode a small VMTI body (with at least one VTargetPack via Tag 101 VTargetSeries — copy a fixture byte sequence from the tst-py test), assert `frameWidth`/`frameHeight`/`targets().size()` + a target's `targetId`; encode round-trip (`encodeVmti` body + `encodeVmtiStandalone` framed, assert standalone starts with `VMTI_LS_UL`); strict missing-required-tag rejection. Use the exact fixture bytes from the tst-py test (cite which test function).

- [ ] **Step 4: Write `bindings/jvm/src/klv/st0903.rs`** — `nDecodeVmti(byte[], boolean strict) -> jobject`, `nEncodeVmti(VmtiLs) -> byte[]`, `nEncodeVmtiStandalone(VmtiLs) -> byte[]`. Port `convert_vmti_ls`/`convert_vtarget_pack`/`py_to_vmti_ls`/`py_to_vtarget_pack` from `bindings/python/src/klv.rs` (incl. the `targets` list iteration, `target_color → TargetColor`, and the typed-tag collision-drop). Build `VmtiLs` + each `VTargetPack` via their Builders.

- [ ] **Step 5: Add facade methods:** `decodeVmti(byte[])`, `decodeVmti(byte[], boolean strict)`, `encodeVmti(VmtiLs)`, `encodeVmtiStandalone(VmtiLs)` + native decls.

- [ ] **Step 6: Build + test.** `./gradlew test --no-daemon --tests 'org.tstrans.klv.St0903Test'` — Expected: PASS.

- [ ] **Step 7: Commit.** `git commit -am "tst-jni klv: ST 0903 VMTI LS + VTargetPack decode+encode"`

---

## Task 4: ST 0601 — UAS Datalink LS (80 fields + composites)

**Source of truth:** `tstrans.klv.{GeoPoint,Attitude,FieldOfView,Corners,UasDatalinkLs}` + `decode_uas_datalink`/`encode_uas_datalink`/`encode_uas_datalink_strict_compliance` (`klv.py`/`klv.rs`); `crates/tst-core/src/klv/st0601/model.rs`; `is_st0601_typed_tag` (`1 | 2 | 65 | 5..=91`).

**Files:** Create `klv/GeoPoint.java`, `klv/Attitude.java`, `klv/FieldOfView.java`, `klv/Corners.java`, `klv/UasDatalinkLs.java` (+Builder + 6 composite accessors), `bindings/jvm/src/klv/st0601.rs`; modify `klv/Klv.java`; Test `klv/St0601Test.java`.

- [ ] **Step 1: Write the 4 composite records.** `GeoPoint(double latDeg, double lonDeg, double altM)`; `Attitude(double headingDeg, double pitchDeg, double rollDeg)`; `FieldOfView(double horizontalDeg, double verticalDeg)`; `Corners(double[] p1, double[] p2, double[] p3, double[] p4)` — OR, to keep value-equality, model corner points as `GeoPoint`-less `record LatLon(double latDeg, double lonDeg)` and `Corners(LatLon p1, LatLon p2, LatLon p3, LatLon p4)`. **Use `LatLon`.** (tst-py uses `tuple[float,float]`; `LatLon` is the value-clean Java analogue.)

- [ ] **Step 2: Write `UasDatalinkLs.java`** — record (`implements KlvSet`) + public Builder, the 80 fields ported field-for-field from `klv.py` `UasDatalinkLs` (`byte[]`/`ByteBuffer universalLabel` 16-byte — store as `ByteBuffer`; `int declaredVersion`; the `String`/`Integer`/`Long`/`Double` optionals; `ByteBuffer securityLocalSet` (Tag 48), `ByteBuffer vmti` (Tag 74); `List<KlvUnknownField> unknown`; `List<KlvFieldError> fieldErrors`). Plus the 6 composite accessor methods returning `Optional<...>` exactly as `klv.py`'s `sensor_position/sensor_attitude/sensor_fov/platform_attitude/frame_center/corners` (incl. the absolute-corners-preferred-then-offset-fallback logic in `corners()`). Compact ctor validates `universalLabel` is 16 bytes.

- [ ] **Step 3: Write `klv/St0601Test.java` (failing).** Port from `bindings/python/tests/test_klv_st0601.py` + `test_klv_st0601_composites.py` + `test_klv_encode_st0601.py`. Minimum: decode a real ST 0601 record (reuse a fixture byte array from the tst-py test or the `crates/tst-core/tests/fixtures/st0601/` synthetic), assert `sensorPosition()`/`frameCenter()` populated; decode→encode→decode field stability; `compliance=true` path; `encodeUasDatalinkStrictCompliance` on a record missing a mandatory tag → `KlvEncodeException(MISSING_MANDATORY_ITEM)`.

- [ ] **Step 4: Write `bindings/jvm/src/klv/st0601.rs`** — `nDecodeUasDatalink(byte[], boolean strict, boolean compliance) -> jobject`, `nEncodeUasDatalink(UasDatalinkLs) -> byte[]`, `nEncodeUasDatalinkStrictCompliance(UasDatalinkLs) -> byte[]`. Port `convert_uas_datalink_ls`/`py_to_uas_datalink_ls` from `bindings/python/src/klv.rs` (the 80-field projection + `unknown` collision-drop + 16-byte UL handling). Build via `UasDatalinkLs$Builder`. (The composite accessors are pure Java — NOT marshalled.)

- [ ] **Step 5: Add facade methods:** `decodeUasDatalink(byte[])`, `decodeUasDatalink(byte[], boolean strict, boolean compliance)`, `encodeUasDatalink(UasDatalinkLs)`, `encodeUasDatalinkStrictCompliance(UasDatalinkLs)` + native decls.

- [ ] **Step 6: Build + test.** `./gradlew test --no-daemon --tests 'org.tstrans.klv.St0601Test'` — Expected: PASS.

- [ ] **Step 7: Commit.** `git commit -am "tst-jni klv: ST 0601 UAS Datalink LS decode+encode + composites"`

---

## Task 5: UL dispatcher + seal `KlvSet` + cross-binding parity + surface-manifest + docs

**Files:** Modify `klv/Klv.java`, `klv/KlvSet.java`; create `klv/ParseUniversalTest.java`; extend `bindings/jvm/src/test/java/org/tstrans/scenarios/ScenarioReproductionTest.java`; modify `tests/coverage/surface-manifest.toml`, `docs/languages/jvm.md`.

- [ ] **Step 1: Seal `KlvSet`.** Change to `public sealed interface KlvSet permits UasDatalinkLs, SecurityLs, PrecisionTimeStampPack, VmtiLs {}`. Confirm each set's `implements KlvSet` already present (Tasks 1–4).

- [ ] **Step 2: Implement `Klv.parseUniversal(byte[]) -> Optional<KlvSet>`** in **pure Java**, mirroring `parse_klv_universal` in `klv.py` (UL match → for body-only sets ST 0102/0903 peel the 16-byte UL + outer BER length then call `decodeSecurity`/`decodeVmti` on the body; for ST 0601/0605 pass the full buffer). Throw `KlvDecodeException(BAD_UNIVERSAL_LABEL)` when `buf.length < 16`; `KlvDecodeException(TRUNCATED_SET)`/`MALFORMED_BYTES` for the BER-peel failures exactly as tst-py raises `KlvError(TRUNCATED_SET/MALFORMED_BYTES)`. Return `Optional.empty()` for an unrecognized UL. Port the `_read_ber_length` helper as a private static Java method.

- [ ] **Step 3: Write `klv/ParseUniversalTest.java`.** Port `bindings/python/tests/test_parse_klv_universal.py`: feed a full ST 0601 record → `instanceof UasDatalinkLs`; a framed ST 0605 pack → `PrecisionTimeStampPack`; a `[SECURITY_LS_UL][BER][body]` → `SecurityLs`; a framed VMTI → `VmtiLs`; an unknown UL → `Optional.empty()`; a <16-byte buffer → `KlvDecodeException(BAD_UNIVERSAL_LABEL)`. (No `switch`-on-sealed — use `instanceof`.)

- [ ] **Step 4: Cross-binding shared-golden reproduction.** Extend `ScenarioReproductionTest.java`: load the `h264-sync-klv-aucell` scenario's Metadata payload bytes (the real ST 0601 UAS UL blob — the demux wave already verified these bytes are byte-identical across Rust/Python/Java via `payload_sha256`), run `Klv.decodeUasDatalink(payload)`, and assert the decoded fields. **Compute the expected field values once** by running the Rust decoder on the same bytes and pin them with a comment naming the source:
```bash
# during implementation, dump expected fields from tst_core to pin in the Java test:
SRT_FORCE_VENDORED=1 cargo run -p tst-core --example extract_klv -- <the aucell payload>  # or a tiny throwaway bin
```
  Pin 2–3 stable scalar fields (e.g. `timestampUs`, a `sensorLatDeg`/`frameCenterLatDeg` if present) as the cross-binding-agreed decode of the shared golden. Add a code comment: "expected = `tst_core::klv::st0601::decode` of the byte-identical `h264-sync-klv-aucell` Metadata payload (cross-binding parity)." If `h264-sync-klv-aucell`'s KLV is a minimal synthetic LS without telemetry, fall back to `h264-st0601-mp` / extend with whichever shared golden carries the richest ST 0601 payload — verify which by reading the golden.json + the scenario's KLV.

- [ ] **Step 5: Graduate surface-manifest `java:` rows.** In `tests/coverage/surface-manifest.toml`, add `java:` columns to the existing `tst_core::klv::st0601::{decode,encode}` rows and add `[[surface]]` rows for the other sets' decode/encode mapped to the Java facade leaves (`java:org.tstrans.klv.Klv.decodeUasDatalink`, `.encodeUasDatalink`, `.decodeSecurity`, `.encodeSecurity`, `.decodePrecisionTimestamp`, `.encodePrecisionTimestamp`, `.decodeVmti`, `.encodeVmti`, `.encodeVmtiStandalone`, `.parseUniversal`). Follow the demux wave's `java:` graduation shape (the resolver greps the last dotted leaf over `bindings/jvm/src/main/java`).

- [ ] **Step 6: Write the `docs/languages/jvm.md` klv section.** Mirror the existing demux/mux sections: a short "Typed KLV" subsection — decode an ST 0601 record, read a composite accessor, encode round-trip, `parseUniversal` dispatch, and the field-error model note. Keep it teaching-quality; no outside-repo paths or codenames.

- [ ] **Step 7: Full local gate.**
```bash
cd bindings/jvm && ./gradlew clean test --no-daemon          # all klv tests + prior suites green
nm -D target/release/libtstjni.so | grep -i klv               # new Java_org_tstrans_klv_* symbols present
cd /home/aklofas/Projects/ts-transformer/ts-transformer
SRT_FORCE_VENDORED=1 cargo clippy -p tst-jni --all-targets -- -D warnings
cargo fmt --all -- --check
bash scripts/check/jvm/error-mapping-coverage.sh </dev/null   # OK
bash scripts/check/repo/surface-manifest.sh </dev/null        # OK
for s in $(find scripts/check -name '*.sh' ! -name 'freertos-srt.sh'); do bash "$s" </dev/null || echo "FAIL: $s"; done
grep -rniE 'aklofas|com\.aklofas|videolink|calfire|courtney|flightops' bindings/jvm || echo "identity clean"
rg -c '#\[non_exhaustive\]' crates/ bindings/ --type rust | awk -F: '{s+=$2} END{print "non_exhaustive:", s}'  # expect 258 (binding-only diff; never write the literal token in a comment)
```
Expected: all green; `non_exhaustive` unchanged at 258.

- [ ] **Step 8: Commit.** `git commit -am "tst-jni klv: UL dispatcher, cross-binding parity, surface-manifest, docs"`

---

## Self-review checklist (controller runs before opening the PR)

1. **Spec coverage:** every `tstrans.klv` entity has a Java analogue — TimeStatus, PrecisionTimeStampPack, 3 ST 0102 enums, SecurityLs, VTargetPack, VmtiLs, 4 ST 0601 composites, UasDatalinkLs, KlvFieldError(Kind), KlvUnknownField, UL constants, isSt0601Family, parseUniversal, all decode/encode entry points, both exception families. ✔ if all present.
2. **Enum codepoints** verified value-for-value against `st0102/enums.rs` (non-contiguous!). ✔
3. **Error mappings** match `klv_decode_error_to_pyerr` / `klv_encode_error_to_pyerr` arm-for-arm; all kind literals appear inline in `error.rs` (ratchet green). ✔
4. **`new_object` / Builder descriptors** verified against each Java type's actual ctor/setter signatures (the keystone's foot-gun). ✔
5. **No `switch`-on-sealed** in committed Java; `KlvSet` sealing proved by `instanceof` / reflection. ✔
6. **`cargo fmt --all -- --check`** run on every new Rust file (edition-2024 import ordering bit all 3 prior waves). ✔
7. **`unknown` collision-drop** predicates ported per set; `decode→encode→decode` round-trips green. ✔

## Ship flow

Feature branch + PR (do NOT push to main, do NOT merge yet). Resolve Copilot threads; poll CI to fully-green via `gh pr view <n> --json statusCheckRollup` (gh run watch is unreliable — see `feedback_gh_run_watch_exit_status_unreliable`); then rebase-merge `--delete-branch` for linear history. Afterward: new `project_tst_jni_klv_shipped.md` memory + MEMORY.md index line + ROADMAP.md (Recently shipped + NEXT pointer → codec wave) + project CLAUDE.md "Next up" update.
