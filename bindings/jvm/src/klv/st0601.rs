//! JNI surface for ST 0601 UAS Datalink LS decode/encode.
//!
//! `nDecodeUasDatalink(byte[], boolean strict, boolean compliance) -> UasDatalinkLs` —
//! dispatches: `compliance=true` → `decode_strict_compliance`; else `strict=true` →
//! `decode_strict`; else `decode` (lenient). Builds the Java `UasDatalinkLs` via its
//! public mutable `Builder` (the Builder-marshalling pattern from Tasks 2–3).
//!
//! `nEncodeUasDatalinkWithPolicy(UasDatalinkLs, int policy) -> byte[]` — reads all
//! fields via accessor `call_method`s, builds a Rust `UasDatalinkLs`, calls
//! `encode_to_vec_with`. The Java 1-arg `encodeUasDatalink(record)` delegates here
//! with `policy=0` (ERROR). The 2-arg overload passes `policy=1` for INDICATOR.
//! Mirrors tst-py's `py_to_uas_datalink_ls` including 16-byte UL validation and
//! the `is_st0601_typed_tag` collision-drop.
//!
//! `nEncodeUasDatalinkStrictCompliance(UasDatalinkLs) -> byte[]` — same read path,
//! calls `encode_strict_compliance` instead.
//!
//! ### JNI local-ref capacity (CRITICAL for 133-field set)
//!
//! `build_uas_datalink` calls `env.ensure_local_capacity(224)` at the top.
//! With 12 String fields + ~110 Double/Long/Integer/ByteBuffer fields (WP-A's
//! 51 new fields pushed the total from 56 to 107; WP-B's 25 new fields + the
//! `imapbSpecials` list pushed it to 133) + builder + lists + JNI scratch,
//! 224 slots safely covers the worst-case fully-populated record.
//! Skipping this call WILL crash the JVM for records with many populated fields.

use jni::JNIEnv;
use jni::objects::{JByteArray, JClass, JObject, JValue};
use jni::sys::{jint, jlong, jobject, jstring};
use tst_core::klv::ImapbSpecial;
use tst_core::klv::st0601::{
    EncodeConfig, IcingDetected, OperationalMode, OutOfRangePolicy, PlatformStatus,
    SensorControlMode, SensorFovName, St0601SentinelMeaning, UasDatalinkLs,
    decode as decode_lenient, decode_strict, decode_strict_compliance, encode_strict_compliance,
    encode_to_vec_with, st0601_sentinel_meaning,
};
use tst_core::klv::universal_label::UniversalLabel;

use crate::error::{map_klv_decode_error, map_klv_encode_error};
use crate::jutil::{
    build_field_errors, build_long_list, build_unknown_list, checked_u8, checked_u16, checked_u32,
    read_byte_buffer, read_nullable_byte_buffer, read_nullable_double, read_nullable_int,
    read_nullable_long, read_nullable_string, read_unknown_list, wrap_heap_byte_buffer,
};

// -----------------------------------------------------------------------
// Builder class / method-descriptor constants
// -----------------------------------------------------------------------

const BUILDER_CLASS: &str = "org/tstrans/klv/UasDatalinkLs$Builder";
const BUILDER_SIG_INT: &str = "(I)Lorg/tstrans/klv/UasDatalinkLs$Builder;";
const BUILDER_SIG_LONG: &str = "(J)Lorg/tstrans/klv/UasDatalinkLs$Builder;";
const BUILDER_SIG_DBL: &str = "(D)Lorg/tstrans/klv/UasDatalinkLs$Builder;";
const BUILDER_SIG_STR: &str = "(Ljava/lang/String;)Lorg/tstrans/klv/UasDatalinkLs$Builder;";
const BUILDER_SIG_BUF: &str = "(Ljava/nio/ByteBuffer;)Lorg/tstrans/klv/UasDatalinkLs$Builder;";
const BUILDER_SIG_LIST: &str = "(Ljava/util/List;)Lorg/tstrans/klv/UasDatalinkLs$Builder;";

/// ST 0601 LS typed + reserved tags — mirrors `tags::TAGS` in
/// `crates/tst-core/src/klv/st0601/tags.rs` (128 entries as of WP-B: 1-65,
/// 67-80, 82-101, 103-114, 117-120, 123-126, 129, 131-137, 139). Tag 66
/// (deprecated-forever), tag 81 (Image Horizon Pixel Pack, a DLP — lands with
/// the WP-C packs), and tag 102 (SDCC-FLP, multi-instance — lands in a later
/// WP), plus 115-116, 121-122, 127-128, 130, 138, 140..=255, are
/// forward-compat/deferred and may legitimately appear in `unknown`.
/// Extended from the WP-A 103-tag set (add: 96, 103-105, 109-114, 117-120,
/// 123-126, 131-134, 136-137, 139) — keep this in sync with `tags::TAGS` when new
/// tags are typed, or a caller-supplied `unknown` entry for a newly-typed
/// tag will slip past this filter and get rejected downstream by the real
/// Rust encoder's own (stricter, canonical) check instead of being
/// silently dropped here per the documented "typed wins" collision policy
/// — mirrors tst-py's `is_st0601_typed_tag` fix.
fn is_st0601_typed_tag(tag: u32) -> bool {
    matches!(
        tag,
        1..=65 | 67..=80 | 82..=101 | 103..=114 | 117..=120 | 123..=126 | 129 | 131..=137 | 139
    )
}

// ---------------------------------------------------------------------------
// ST 0601 — Tags 34/63/77 coded enums (WP-A Table A3)
// ---------------------------------------------------------------------------
//
// `IcingDetected::to_wire`/`from_wire` (and the SensorFovName /
// OperationalMode equivalents) are `pub(crate)`-scoped to tst-core, so the
// tiny wire-code tables are duplicated locally here — same rationale as the
// `is_st0601_typed_tag` predicate above (narrow inventories kept local rather
// than threading internal Rust APIs out of tst-core). Mirrors tst-py's
// `convert_icing_detected`/`icing_detected_from_wire` (and the SensorFovName /
// OperationalMode equivalents), adapted to the JNI raw-codepoint-`Integer`
// crossing (no Python-enum-instance step).

/// Extract the ST 0601.19 §8.34 wire codepoint from an `IcingDetected`.
fn icing_detected_to_code(v: IcingDetected) -> u8 {
    match v {
        IcingDetected::DetectorOff => 0,
        IcingDetected::NoIcingDetected => 1,
        IcingDetected::IcingDetected => 2,
        IcingDetected::Other(b) => b,
        // `#[non_exhaustive]` in tst-core forces this wildcard even though
        // every current variant is matched above; unreachable in practice.
        _ => unreachable!("tst-core added an IcingDetected variant not yet mirrored in tst-jni"),
    }
}

/// Inverse of [`icing_detected_to_code`].
fn icing_detected_from_code(b: u8) -> IcingDetected {
    match b {
        0 => IcingDetected::DetectorOff,
        1 => IcingDetected::NoIcingDetected,
        2 => IcingDetected::IcingDetected,
        other => IcingDetected::Other(other),
    }
}

/// Extract the ST 0601.19 §8.63 wire codepoint from a `SensorFovName`.
fn sensor_fov_name_to_code(v: SensorFovName) -> u8 {
    match v {
        SensorFovName::Ultranarrow => 0,
        SensorFovName::Narrow => 1,
        SensorFovName::Medium => 2,
        SensorFovName::Wide => 3,
        SensorFovName::Ultrawide => 4,
        SensorFovName::NarrowMedium => 5,
        SensorFovName::TwoXUltranarrow => 6,
        SensorFovName::FourXUltranarrow => 7,
        SensorFovName::ContinuousZoom => 8,
        SensorFovName::Other(b) => b,
        _ => unreachable!("tst-core added a SensorFovName variant not yet mirrored in tst-jni"),
    }
}

/// Inverse of [`sensor_fov_name_to_code`].
fn sensor_fov_name_from_code(b: u8) -> SensorFovName {
    match b {
        0 => SensorFovName::Ultranarrow,
        1 => SensorFovName::Narrow,
        2 => SensorFovName::Medium,
        3 => SensorFovName::Wide,
        4 => SensorFovName::Ultrawide,
        5 => SensorFovName::NarrowMedium,
        6 => SensorFovName::TwoXUltranarrow,
        7 => SensorFovName::FourXUltranarrow,
        8 => SensorFovName::ContinuousZoom,
        other => SensorFovName::Other(other),
    }
}

/// Extract the ST 0601.19 §8.77 wire codepoint from an `OperationalMode`.
fn operational_mode_to_code(v: OperationalMode) -> u8 {
    match v {
        OperationalMode::OtherMode => 0,
        OperationalMode::Operational => 1,
        OperationalMode::Training => 2,
        OperationalMode::Exercise => 3,
        OperationalMode::Maintenance => 4,
        OperationalMode::Test => 5,
        OperationalMode::Other(b) => b,
        _ => unreachable!("tst-core added an OperationalMode variant not yet mirrored in tst-jni"),
    }
}

/// Inverse of [`operational_mode_to_code`].
fn operational_mode_from_code(b: u8) -> OperationalMode {
    match b {
        0 => OperationalMode::OtherMode,
        1 => OperationalMode::Operational,
        2 => OperationalMode::Training,
        3 => OperationalMode::Exercise,
        4 => OperationalMode::Maintenance,
        5 => OperationalMode::Test,
        other => OperationalMode::Other(other),
    }
}

// ---------------------------------------------------------------------------
// ST 0601 — Tags 125/126 coded enums (WP-B Table B2)
// ---------------------------------------------------------------------------

/// Extract the ST 0601.19 §8.125 wire codepoint from a `PlatformStatus`.
fn platform_status_to_code(v: PlatformStatus) -> u8 {
    match v {
        PlatformStatus::Active => 0,
        PlatformStatus::PreFlight => 1,
        PlatformStatus::PreFlightTaxiing => 2,
        PlatformStatus::RunUp => 3,
        PlatformStatus::TakeOff => 4,
        PlatformStatus::Ingress => 5,
        PlatformStatus::ManualOperation => 6,
        PlatformStatus::AutomatedOrbit => 7,
        PlatformStatus::Transitioning => 8,
        PlatformStatus::Egress => 9,
        PlatformStatus::Landing => 10,
        PlatformStatus::LandedTaxiing => 11,
        PlatformStatus::LandedParked => 12,
        PlatformStatus::Other(b) => b,
        // `#[non_exhaustive]` in tst-core forces this wildcard even though
        // every current variant is matched above; unreachable in practice.
        _ => unreachable!("tst-core added a PlatformStatus variant not yet mirrored in tst-jni"),
    }
}

/// Inverse of [`platform_status_to_code`].
fn platform_status_from_code(b: u8) -> PlatformStatus {
    match b {
        0 => PlatformStatus::Active,
        1 => PlatformStatus::PreFlight,
        2 => PlatformStatus::PreFlightTaxiing,
        3 => PlatformStatus::RunUp,
        4 => PlatformStatus::TakeOff,
        5 => PlatformStatus::Ingress,
        6 => PlatformStatus::ManualOperation,
        7 => PlatformStatus::AutomatedOrbit,
        8 => PlatformStatus::Transitioning,
        9 => PlatformStatus::Egress,
        10 => PlatformStatus::Landing,
        11 => PlatformStatus::LandedTaxiing,
        12 => PlatformStatus::LandedParked,
        other => PlatformStatus::Other(other),
    }
}

/// Extract the ST 0601.19 §8.126 wire codepoint from a `SensorControlMode`.
fn sensor_control_mode_to_code(v: SensorControlMode) -> u8 {
    match v {
        SensorControlMode::Off => 0,
        SensorControlMode::HomePosition => 1,
        SensorControlMode::Uncontrolled => 2,
        SensorControlMode::ManualControl => 3,
        SensorControlMode::Calibrating => 4,
        SensorControlMode::AutoHoldingPosition => 5,
        SensorControlMode::AutoTracking => 6,
        SensorControlMode::Other(b) => b,
        _ => {
            unreachable!("tst-core added a SensorControlMode variant not yet mirrored in tst-jni")
        }
    }
}

/// Inverse of [`sensor_control_mode_to_code`].
fn sensor_control_mode_from_code(b: u8) -> SensorControlMode {
    match b {
        0 => SensorControlMode::Off,
        1 => SensorControlMode::HomePosition,
        2 => SensorControlMode::Uncontrolled,
        3 => SensorControlMode::ManualControl,
        4 => SensorControlMode::Calibrating,
        5 => SensorControlMode::AutoHoldingPosition,
        6 => SensorControlMode::AutoTracking,
        other => SensorControlMode::Other(other),
    }
}

// ---------------------------------------------------------------------------
// ST 1201.5 — imapb_specials side channel (WP-B)
// ---------------------------------------------------------------------------
//
// Crossing shape (DECIDED, shared with the Python binding): a
// `List<ImapbSpecialEntry>` of `(tag: int, code: String, payload: long)`
// entries. `code` names the `ImapbSpecial` family; `payload` is the
// NaN-id/signal value (0 for the payload-less codes). Mirrors tst-py's
// `imapb_special_to_code`/`imapb_special_from_code`.

/// Translate a Rust `ImapbSpecial` to its `(code, payload)` wire-string
/// pair. Throws `IllegalArgumentException` (rather than silently
/// mislabeling) on a future non-exhaustive variant — same stance as
/// `platform_status_to_code`/`sensor_control_mode_to_code` above.
fn imapb_special_to_code(
    env: &mut JNIEnv,
    s: ImapbSpecial,
) -> jni::errors::Result<(&'static str, u64)> {
    Ok(match s {
        ImapbSpecial::BelowMin => ("below_min", 0),
        ImapbSpecial::AboveMax => ("above_max", 0),
        ImapbSpecial::PositiveInfinity => ("pos_infinity", 0),
        ImapbSpecial::NegativeInfinity => ("neg_infinity", 0),
        ImapbSpecial::PositiveQuietNaN { nan_id } => ("pos_quiet_nan", nan_id),
        ImapbSpecial::NegativeQuietNaN { nan_id } => ("neg_quiet_nan", nan_id),
        ImapbSpecial::PositiveSignalingNaN { signal } => ("pos_signaling_nan", signal),
        ImapbSpecial::NegativeSignalingNaN { signal } => ("neg_signaling_nan", signal),
        ImapbSpecial::UserDefined { signal } => ("user_defined", signal),
        // `#[non_exhaustive]` in tst-core: no current variant reaches here.
        _ => {
            let _ = env.throw_new(
                "java/lang/IllegalArgumentException",
                "unknown ImapbSpecial variant crossing the binding",
            );
            return Err(jni::errors::Error::JavaException);
        }
    })
}

/// Inverse of [`imapb_special_to_code`]. Throws `IllegalArgumentException`
/// for a code string outside the 9-member set (audit #6 "validate-don't-drop"
/// stance — same as the Python `imapb_special_from_code`).
fn imapb_special_from_code(
    env: &mut JNIEnv,
    code: &str,
    payload: u64,
) -> jni::errors::Result<ImapbSpecial> {
    Ok(match code {
        "below_min" => ImapbSpecial::BelowMin,
        "above_max" => ImapbSpecial::AboveMax,
        "pos_infinity" => ImapbSpecial::PositiveInfinity,
        "neg_infinity" => ImapbSpecial::NegativeInfinity,
        "pos_quiet_nan" => ImapbSpecial::PositiveQuietNaN { nan_id: payload },
        "neg_quiet_nan" => ImapbSpecial::NegativeQuietNaN { nan_id: payload },
        "pos_signaling_nan" => ImapbSpecial::PositiveSignalingNaN { signal: payload },
        "neg_signaling_nan" => ImapbSpecial::NegativeSignalingNaN { signal: payload },
        "user_defined" => ImapbSpecial::UserDefined { signal: payload },
        other => {
            let _ = env.throw_new(
                "java/lang/IllegalArgumentException",
                format!(
                    "unknown imapbSpecials code {other:?}; expected one of below_min, above_max, \
                     pos_infinity, neg_infinity, pos_quiet_nan, neg_quiet_nan, pos_signaling_nan, \
                     neg_signaling_nan, user_defined"
                ),
            );
            return Err(jni::errors::Error::JavaException);
        }
    })
}

/// Range-check a Java `int` value against the i8 range, then narrow. Throws
/// `IllegalArgumentException` and returns `Err(JavaException)` on overflow —
/// local to this module (mirrors `jutil::checked_u8`'s idiom) since ST 0601 has
/// exactly one i8-typed field (Tag 39, `outside_air_temp_c`).
fn checked_i8(env: &mut JNIEnv, value: i64, field: &str) -> jni::errors::Result<i8> {
    if !(-128..=127).contains(&value) {
        let _ = env.throw_new(
            "java/lang/IllegalArgumentException",
            format!("{field} must be -128..=127, got {value}"),
        );
        return Err(jni::errors::Error::JavaException);
    }
    Ok(value as i8)
}

// -----------------------------------------------------------------------
// Decode entry point
// -----------------------------------------------------------------------

/// `org.tstrans.klv.Klv.nDecodeUasDatalink(byte[], boolean strict, boolean compliance)`
///
/// Decodes a full ST 0601 record (full buffer including the 16-byte UL).
/// Dispatches: compliance → `decode_strict_compliance`; strict → `decode_strict`;
/// else → lenient `decode`. On success, builds and returns a Java `UasDatalinkLs`
/// via its `Builder`. On failure, throws a `KlvDecodeException` and returns null.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_klv_Klv_nDecodeUasDatalink<'local>(
    mut env: JNIEnv<'local>,
    _c: JClass<'local>,
    buf: JByteArray<'local>,
    strict: jni::sys::jboolean,
    compliance: jni::sys::jboolean,
) -> jobject {
    crate::panic::jni_catch(&mut env, std::ptr::null_mut(), |env| {
        let bytes = match env.convert_byte_array(&buf) {
            Ok(b) => b,
            Err(e) => {
                let _ = env.throw_new(
                    "java/lang/RuntimeException",
                    format!("nDecodeUasDatalink: byte[] read failed: {e}"),
                );
                return JObject::null().into_raw();
            }
        };
        let result = if compliance != 0 {
            decode_strict_compliance(&bytes)
        } else if strict != 0 {
            decode_strict(&bytes)
        } else {
            decode_lenient(&bytes)
        };
        match result {
            Ok(rec) => match build_uas_datalink(env, &rec) {
                Ok(raw) => raw,
                Err(_) => JObject::null().into_raw(),
            },
            Err(e) => {
                map_klv_decode_error(env, &e);
                JObject::null().into_raw()
            }
        }
    })
}

// -----------------------------------------------------------------------
// Encode entry points
// -----------------------------------------------------------------------

/// `org.tstrans.klv.Klv.nEncodeUasDatalinkWithPolicy(UasDatalinkLs, int policy) -> byte[]`
///
/// Reads all fields from the Java `UasDatalinkLs` record, builds a Rust
/// `UasDatalinkLs`, then calls [`encode_to_vec_with`] using an
/// [`OutOfRangePolicy`] mapped from the `policy` int:
/// - `0` → [`OutOfRangePolicy::Error`] (throws on any out-of-range value)
/// - `1` → [`OutOfRangePolicy::Indicator`] (emits the spec's Out-of-Range
///   special for eligible linear-range tags: 6, 7, 50, 51, 52, 79, 80,
///   90–93; separately, WP-B's 14 IMAPB tags — 96, 103-105, 109,
///   112-114, 117-120, 132, 134 — get their own ST 1201.5 BelowMin/AboveMax
///   special instead of the INT_MIN sentinel)
///
/// Any other ordinal throws `java.lang.IllegalArgumentException` — this
/// signals enum drift between the Java and Rust layers.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_klv_Klv_nEncodeUasDatalinkWithPolicy<'local>(
    mut env: JNIEnv<'local>,
    _c: JClass<'local>,
    record: JObject<'local>,
    policy: jint,
) -> jobject {
    crate::panic::jni_catch(&mut env, std::ptr::null_mut(), |env| {
        // Exact-ordinal validation: out-of-range means enum drift between
        // the Java and Rust layers — fail loudly.
        let oor_policy = match policy {
            0 => OutOfRangePolicy::Error,
            1 => OutOfRangePolicy::Indicator,
            other => {
                let _ = env.throw_new(
                    "java/lang/IllegalArgumentException",
                    format!("unknown OutOfRangePolicy ordinal {other}"),
                );
                return JObject::null().into_raw();
            }
        };
        let mut opts = EncodeConfig::default();
        opts.out_of_range_policy = oor_policy;
        match read_uas_datalink(env, &record) {
            Ok(rust_rec) => match encode_to_vec_with(&rust_rec, &opts) {
                Ok(bytes) => match env.byte_array_from_slice(&bytes) {
                    Ok(arr) => arr.into_raw(),
                    Err(e) => {
                        let _ = env.throw_new(
                            "java/lang/RuntimeException",
                            format!(
                                "nEncodeUasDatalinkWithPolicy: byte_array_from_slice failed: {e}"
                            ),
                        );
                        JObject::null().into_raw()
                    }
                },
                Err(e) => {
                    map_klv_encode_error(env, &e);
                    JObject::null().into_raw()
                }
            },
            Err(e) => {
                if !env.exception_check().unwrap_or(false) {
                    let _ = env.throw_new(
                        "java/lang/RuntimeException",
                        format!("nEncodeUasDatalinkWithPolicy: field read failed: {e}"),
                    );
                }
                JObject::null().into_raw()
            }
        }
    })
}

/// `org.tstrans.klv.Klv.nEncodeUasDatalinkStrictCompliance(UasDatalinkLs) -> byte[]`
///
/// Reads all fields from the Java `UasDatalinkLs` record, builds a Rust
/// `UasDatalinkLs`, calls `encode_strict_compliance`. Enforces mandatory-tag
/// presence (Tag 2 / Tag 65 / Tag 1) and structural ordering. Mirrors tst-py's
/// `encode_uas_datalink_strict_compliance`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_klv_Klv_nEncodeUasDatalinkStrictCompliance<'local>(
    mut env: JNIEnv<'local>,
    _c: JClass<'local>,
    record: JObject<'local>,
) -> jobject {
    crate::panic::jni_catch(
        &mut env,
        std::ptr::null_mut(),
        |env| match read_uas_datalink(env, &record) {
            Ok(rust_rec) => match encode_strict_compliance(&rust_rec) {
                Ok(bytes) => match env.byte_array_from_slice(&bytes) {
                    Ok(arr) => arr.into_raw(),
                    Err(e) => {
                        let _ = env.throw_new(
                            "java/lang/RuntimeException",
                            format!(
                                "nEncodeUasDatalinkStrictCompliance: byte_array_from_slice failed: {e}"
                            ),
                        );
                        JObject::null().into_raw()
                    }
                },
                Err(e) => {
                    map_klv_encode_error(env, &e);
                    JObject::null().into_raw()
                }
            },
            Err(e) => {
                if !env.exception_check().unwrap_or(false) {
                    let _ = env.throw_new(
                        "java/lang/RuntimeException",
                        format!("nEncodeUasDatalinkStrictCompliance: field read failed: {e}"),
                    );
                }
                JObject::null().into_raw()
            }
        },
    )
}

// -----------------------------------------------------------------------
// Sentinel-meaning lookup
// -----------------------------------------------------------------------

/// `org.tstrans.klv.Klv.nSt0601SentinelMeaning(long tag) -> String | null`
///
/// Delegates to [`st0601_sentinel_meaning`] (the ST 0601.19 §7.5 special-value
/// assignments), mapping the [`St0601SentinelMeaning`] variants to the same
/// strings tst-py returns: `OutOfRange` → `"out_of_range"`, `Reserved` →
/// `"reserved"`, `NotAvailable` → `"not_available"`, no assignment → null.
/// Total over its input: a tag outside the u32 range simply has no assigned
/// meaning (null) — the only failure path is JNI string allocation.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_klv_Klv_nSt0601SentinelMeaning<'local>(
    mut env: JNIEnv<'local>,
    _c: JClass<'local>,
    tag: jlong,
) -> jstring {
    crate::panic::jni_catch(&mut env, std::ptr::null_mut(), |env| {
        let meaning = u32::try_from(tag).ok().and_then(st0601_sentinel_meaning);
        let name = match meaning {
            Some(St0601SentinelMeaning::OutOfRange) => "out_of_range",
            Some(St0601SentinelMeaning::Reserved) => "reserved",
            Some(St0601SentinelMeaning::NotAvailable) => "not_available",
            // `#[non_exhaustive]` in tst-core: no current variant reaches the
            // wildcard; a future variant surfaces as null until mirrored here.
            Some(_) | None => return std::ptr::null_mut(),
        };
        match env.new_string(name) {
            Ok(s) => s.into_raw(),
            Err(e) => {
                let _ = env.throw_new(
                    "java/lang/RuntimeException",
                    format!("nSt0601SentinelMeaning: new_string failed: {e}"),
                );
                std::ptr::null_mut()
            }
        }
    })
}

// -----------------------------------------------------------------------
// Rust → Java builder (decode path)
// -----------------------------------------------------------------------

/// Build a `org.tstrans.klv.UasDatalinkLs` Java record from a Rust `UasDatalinkLs`
/// via the public mutable `Builder`. Mirrors `convert_uas_datalink_ls` in tst-py.
///
/// ### Local-ref capacity (MANDATORY)
///
/// Calls `env.ensure_local_capacity(224)` at the top. With 133 fields (12
/// Strings, ~83 Doubles, ~11 ByteBuffers, ~14 Integers, ~7 Longs, an
/// `imapbSpecials` list, builder + lists), the default ~16-slot JNI local
/// table is completely inadequate. 224 slots is the minimum safe value for
/// a fully populated ST 0601 record (bumped from 192 when WP-B added 25
/// fields + the `imapbSpecials` list; 192 was bumped from 128 when WP-A
/// added 51 fields).
fn build_uas_datalink(env: &mut JNIEnv<'_>, r: &UasDatalinkLs) -> jni::errors::Result<jobject> {
    // CRITICAL: must be called before any new_string / new_object below.
    // 224 slots covers 133 fields + builder + lists + JNI scratch.
    env.ensure_local_capacity(224)?;

    let b = env.new_object(BUILDER_CLASS, "()V", &[])?;

    // --- universal_label: UniversalLabel([u8;16]) → heap ByteBuffer (non-optional) ---
    {
        let ul_buf = wrap_heap_byte_buffer(env, &r.universal_label.0)
            .map_err(|()| jni::errors::Error::JavaException)?;
        env.call_method(
            &b,
            "universalLabel",
            BUILDER_SIG_BUF,
            &[JValue::Object(&ul_buf)],
        )?;
    }

    // --- declared_version: u8 → int (non-optional) ---
    env.call_method(
        &b,
        "declaredVersion",
        BUILDER_SIG_INT,
        &[JValue::Int(i32::from(r.declared_version))],
    )?;

    // --- Identity: Optional<String> fields ---
    if let Some(ref v) = r.mission_id {
        let j = env.new_string(v)?;
        env.call_method(&b, "missionId", BUILDER_SIG_STR, &[JValue::Object(&j)])?;
    }
    if let Some(ref v) = r.platform_tail_number {
        let j = env.new_string(v)?;
        env.call_method(
            &b,
            "platformTailNumber",
            BUILDER_SIG_STR,
            &[JValue::Object(&j)],
        )?;
    }
    if let Some(ref v) = r.platform_designation {
        let j = env.new_string(v)?;
        env.call_method(
            &b,
            "platformDesignation",
            BUILDER_SIG_STR,
            &[JValue::Object(&j)],
        )?;
    }
    if let Some(ref v) = r.image_source_sensor {
        let j = env.new_string(v)?;
        env.call_method(
            &b,
            "imageSourceSensor",
            BUILDER_SIG_STR,
            &[JValue::Object(&j)],
        )?;
    }
    if let Some(ref v) = r.image_coordinate_system {
        let j = env.new_string(v)?;
        env.call_method(
            &b,
            "imageCoordinateSystem",
            BUILDER_SIG_STR,
            &[JValue::Object(&j)],
        )?;
    }
    if let Some(ref v) = r.platform_call_sign {
        let j = env.new_string(v)?;
        env.call_method(
            &b,
            "platformCallSign",
            BUILDER_SIG_STR,
            &[JValue::Object(&j)],
        )?;
    }

    // --- Optional<u8> → int ---
    if let Some(v) = r.uas_ls_version {
        env.call_method(
            &b,
            "uasLsVersion",
            BUILDER_SIG_INT,
            &[JValue::Int(i32::from(v))],
        )?;
    }

    // --- Optional<u64> → long ---
    if let Some(v) = r.timestamp_us {
        env.call_method(
            &b,
            "timestampUs",
            BUILDER_SIG_LONG,
            &[JValue::Long(v as i64)],
        )?;
    }

    // --- Platform state: Optional<f64> → double ---
    if let Some(v) = r.platform_heading_deg {
        env.call_method(
            &b,
            "platformHeadingDeg",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }
    if let Some(v) = r.platform_pitch_deg {
        env.call_method(
            &b,
            "platformPitchDeg",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }
    if let Some(v) = r.platform_roll_deg {
        env.call_method(&b, "platformRollDeg", BUILDER_SIG_DBL, &[JValue::Double(v)])?;
    }
    if let Some(v) = r.platform_true_airspeed {
        env.call_method(
            &b,
            "platformTrueAirspeed",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }
    if let Some(v) = r.platform_indicated_airspeed {
        env.call_method(
            &b,
            "platformIndicatedAirspeed",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }
    if let Some(v) = r.platform_pitch_full_deg {
        env.call_method(
            &b,
            "platformPitchFullDeg",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }
    if let Some(v) = r.platform_roll_full_deg {
        env.call_method(
            &b,
            "platformRollFullDeg",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }
    if let Some(v) = r.platform_angle_of_attack_deg {
        env.call_method(
            &b,
            "platformAngleOfAttackDeg",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }

    // --- Sensor pose & position: Optional<f64> → double ---
    if let Some(v) = r.sensor_lat_deg {
        env.call_method(&b, "sensorLatDeg", BUILDER_SIG_DBL, &[JValue::Double(v)])?;
    }
    if let Some(v) = r.sensor_lon_deg {
        env.call_method(&b, "sensorLonDeg", BUILDER_SIG_DBL, &[JValue::Double(v)])?;
    }
    if let Some(v) = r.sensor_alt_m {
        env.call_method(&b, "sensorAltM", BUILDER_SIG_DBL, &[JValue::Double(v)])?;
    }
    if let Some(v) = r.sensor_ellipsoid_height_m {
        env.call_method(
            &b,
            "sensorEllipsoidHeightM",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }
    if let Some(v) = r.sensor_hfov_deg {
        env.call_method(&b, "sensorHfovDeg", BUILDER_SIG_DBL, &[JValue::Double(v)])?;
    }
    if let Some(v) = r.sensor_vfov_deg {
        env.call_method(&b, "sensorVfovDeg", BUILDER_SIG_DBL, &[JValue::Double(v)])?;
    }
    if let Some(v) = r.sensor_rel_az_deg {
        env.call_method(&b, "sensorRelAzDeg", BUILDER_SIG_DBL, &[JValue::Double(v)])?;
    }
    if let Some(v) = r.sensor_rel_el_deg {
        env.call_method(&b, "sensorRelElDeg", BUILDER_SIG_DBL, &[JValue::Double(v)])?;
    }
    if let Some(v) = r.sensor_rel_roll_deg {
        env.call_method(
            &b,
            "sensorRelRollDeg",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }

    // --- Ranging & frame center ---
    if let Some(v) = r.slant_range_m {
        env.call_method(&b, "slantRangeM", BUILDER_SIG_DBL, &[JValue::Double(v)])?;
    }
    if let Some(v) = r.target_width_m {
        env.call_method(&b, "targetWidthM", BUILDER_SIG_DBL, &[JValue::Double(v)])?;
    }
    if let Some(v) = r.frame_center_lat_deg {
        env.call_method(
            &b,
            "frameCenterLatDeg",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }
    if let Some(v) = r.frame_center_lon_deg {
        env.call_method(
            &b,
            "frameCenterLonDeg",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }
    if let Some(v) = r.frame_center_elev_m {
        env.call_method(
            &b,
            "frameCenterElevM",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }
    if let Some(v) = r.frame_center_ellipsoid_height_m {
        env.call_method(
            &b,
            "frameCenterEllipsoidHeightM",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }

    // --- Image corner offsets (tags 26–33) ---
    if let Some(v) = r.corner_lat_offset_p1_deg {
        env.call_method(
            &b,
            "cornerLatOffsetP1Deg",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }
    if let Some(v) = r.corner_lon_offset_p1_deg {
        env.call_method(
            &b,
            "cornerLonOffsetP1Deg",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }
    if let Some(v) = r.corner_lat_offset_p2_deg {
        env.call_method(
            &b,
            "cornerLatOffsetP2Deg",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }
    if let Some(v) = r.corner_lon_offset_p2_deg {
        env.call_method(
            &b,
            "cornerLonOffsetP2Deg",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }
    if let Some(v) = r.corner_lat_offset_p3_deg {
        env.call_method(
            &b,
            "cornerLatOffsetP3Deg",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }
    if let Some(v) = r.corner_lon_offset_p3_deg {
        env.call_method(
            &b,
            "cornerLonOffsetP3Deg",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }
    if let Some(v) = r.corner_lat_offset_p4_deg {
        env.call_method(
            &b,
            "cornerLatOffsetP4Deg",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }
    if let Some(v) = r.corner_lon_offset_p4_deg {
        env.call_method(
            &b,
            "cornerLonOffsetP4Deg",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }

    // --- Image corners full lat/lon (tags 82–89) ---
    if let Some(v) = r.corner_lat_p1_deg {
        env.call_method(&b, "cornerLatP1Deg", BUILDER_SIG_DBL, &[JValue::Double(v)])?;
    }
    if let Some(v) = r.corner_lon_p1_deg {
        env.call_method(&b, "cornerLonP1Deg", BUILDER_SIG_DBL, &[JValue::Double(v)])?;
    }
    if let Some(v) = r.corner_lat_p2_deg {
        env.call_method(&b, "cornerLatP2Deg", BUILDER_SIG_DBL, &[JValue::Double(v)])?;
    }
    if let Some(v) = r.corner_lon_p2_deg {
        env.call_method(&b, "cornerLonP2Deg", BUILDER_SIG_DBL, &[JValue::Double(v)])?;
    }
    if let Some(v) = r.corner_lat_p3_deg {
        env.call_method(&b, "cornerLatP3Deg", BUILDER_SIG_DBL, &[JValue::Double(v)])?;
    }
    if let Some(v) = r.corner_lon_p3_deg {
        env.call_method(&b, "cornerLonP3Deg", BUILDER_SIG_DBL, &[JValue::Double(v)])?;
    }
    if let Some(v) = r.corner_lat_p4_deg {
        env.call_method(&b, "cornerLatP4Deg", BUILDER_SIG_DBL, &[JValue::Double(v)])?;
    }
    if let Some(v) = r.corner_lon_p4_deg {
        env.call_method(&b, "cornerLonP4Deg", BUILDER_SIG_DBL, &[JValue::Double(v)])?;
    }

    // --- Misc ---
    if let Some(v) = r.generic_flag_data {
        env.call_method(
            &b,
            "genericFlagData",
            BUILDER_SIG_INT,
            &[JValue::Int(i32::from(v))],
        )?;
    }

    // Tag 48 — securityLocalSet (Vec<u8> → heap ByteBuffer)
    if let Some(ref bs) = r.security_local_set {
        let buf = wrap_heap_byte_buffer(env, bs).map_err(|()| jni::errors::Error::JavaException)?;
        env.call_method(
            &b,
            "securityLocalSet",
            BUILDER_SIG_BUF,
            &[JValue::Object(&buf)],
        )?;
    }

    // Tag 74 — vmti (Vec<u8> → heap ByteBuffer)
    if let Some(ref bs) = r.vmti {
        let buf = wrap_heap_byte_buffer(env, bs).map_err(|()| jni::errors::Error::JavaException)?;
        env.call_method(&b, "vmti", BUILDER_SIG_BUF, &[JValue::Object(&buf)])?;
    }

    // Tag 94 — miisCoreId (Vec<u8> → byte[])
    if let Some(ref bs) = r.miis_core_id {
        let arr = env.byte_array_from_slice(bs)?;
        env.call_method(
            &b,
            "miisCoreId",
            "([B)Lorg/tstrans/klv/UasDatalinkLs$Builder;",
            &[JValue::Object(&arr)],
        )?;
    }

    // --- WP-A Table A1: ranged f64 fields (tags 35-93 subset) → double ---
    if let Some(v) = r.target_location_lat_deg {
        env.call_method(
            &b,
            "targetLocationLatDeg",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }
    if let Some(v) = r.target_location_lon_deg {
        env.call_method(
            &b,
            "targetLocationLonDeg",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }
    if let Some(v) = r.target_location_elev_m {
        env.call_method(
            &b,
            "targetLocationElevM",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }
    if let Some(v) = r.target_track_gate_width_px {
        env.call_method(
            &b,
            "targetTrackGateWidthPx",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }
    if let Some(v) = r.target_track_gate_height_px {
        env.call_method(
            &b,
            "targetTrackGateHeightPx",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }
    if let Some(v) = r.target_error_ce90_m {
        env.call_method(
            &b,
            "targetErrorCe90M",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }
    if let Some(v) = r.target_error_le90_m {
        env.call_method(
            &b,
            "targetErrorLe90M",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }
    if let Some(v) = r.wind_direction_deg {
        env.call_method(
            &b,
            "windDirectionDeg",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }
    if let Some(v) = r.wind_speed {
        env.call_method(&b, "windSpeed", BUILDER_SIG_DBL, &[JValue::Double(v)])?;
    }
    if let Some(v) = r.static_pressure_mbar {
        env.call_method(
            &b,
            "staticPressureMbar",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }
    if let Some(v) = r.density_altitude_m {
        env.call_method(
            &b,
            "densityAltitudeM",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }
    if let Some(v) = r.differential_pressure_mbar {
        env.call_method(
            &b,
            "differentialPressureMbar",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }
    if let Some(v) = r.airfield_barometric_pressure_mbar {
        env.call_method(
            &b,
            "airfieldBarometricPressureMbar",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }
    if let Some(v) = r.airfield_elevation_m {
        env.call_method(
            &b,
            "airfieldElevationM",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }
    if let Some(v) = r.relative_humidity_pct {
        env.call_method(
            &b,
            "relativeHumidityPct",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }
    if let Some(v) = r.platform_vertical_speed {
        env.call_method(
            &b,
            "platformVerticalSpeed",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }
    if let Some(v) = r.platform_sideslip_deg {
        env.call_method(
            &b,
            "platformSideslipDeg",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }
    if let Some(v) = r.platform_ground_speed {
        env.call_method(
            &b,
            "platformGroundSpeed",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }
    if let Some(v) = r.ground_range_m {
        env.call_method(&b, "groundRangeM", BUILDER_SIG_DBL, &[JValue::Double(v)])?;
    }
    if let Some(v) = r.platform_fuel_remaining_kg {
        env.call_method(
            &b,
            "platformFuelRemainingKg",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }
    if let Some(v) = r.platform_magnetic_heading_deg {
        env.call_method(
            &b,
            "platformMagneticHeadingDeg",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }
    if let Some(v) = r.platform_angle_of_attack_full_deg {
        env.call_method(
            &b,
            "platformAngleOfAttackFullDeg",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }
    if let Some(v) = r.platform_sideslip_full_deg {
        env.call_method(
            &b,
            "platformSideslipFullDeg",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }
    if let Some(v) = r.alternate_platform_lat_deg {
        env.call_method(
            &b,
            "alternatePlatformLatDeg",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }
    if let Some(v) = r.alternate_platform_lon_deg {
        env.call_method(
            &b,
            "alternatePlatformLonDeg",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }
    if let Some(v) = r.alternate_platform_alt_m {
        env.call_method(
            &b,
            "alternatePlatformAltM",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }
    if let Some(v) = r.alternate_platform_heading_deg {
        env.call_method(
            &b,
            "alternatePlatformHeadingDeg",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }
    if let Some(v) = r.alternate_platform_ellipsoid_height_m {
        env.call_method(
            &b,
            "alternatePlatformEllipsoidHeightM",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }
    if let Some(v) = r.sensor_north_velocity {
        env.call_method(
            &b,
            "sensorNorthVelocity",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }
    if let Some(v) = r.sensor_east_velocity {
        env.call_method(
            &b,
            "sensorEastVelocity",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }

    // --- WP-B Table B1: IMAPB f64 fields (tags 96, 103-105, 109, 112-114, 117-120, 132, 134) → double ---
    if let Some(v) = r.target_width_extended_m {
        env.call_method(
            &b,
            "targetWidthExtendedM",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }
    if let Some(v) = r.density_altitude_extended_m {
        env.call_method(
            &b,
            "densityAltitudeExtendedM",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }
    if let Some(v) = r.sensor_ellipsoid_height_extended_m {
        env.call_method(
            &b,
            "sensorEllipsoidHeightExtendedM",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }
    if let Some(v) = r.alternate_platform_ellipsoid_height_extended_m {
        env.call_method(
            &b,
            "alternatePlatformEllipsoidHeightExtendedM",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }
    if let Some(v) = r.range_to_recovery_km {
        env.call_method(
            &b,
            "rangeToRecoveryKm",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }
    if let Some(v) = r.platform_course_angle_deg {
        env.call_method(
            &b,
            "platformCourseAngleDeg",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }
    if let Some(v) = r.altitude_agl_m {
        env.call_method(&b, "altitudeAglM", BUILDER_SIG_DBL, &[JValue::Double(v)])?;
    }
    if let Some(v) = r.radar_altimeter_m {
        env.call_method(&b, "radarAltimeterM", BUILDER_SIG_DBL, &[JValue::Double(v)])?;
    }
    if let Some(v) = r.sensor_azimuth_rate_dps {
        env.call_method(
            &b,
            "sensorAzimuthRateDps",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }
    if let Some(v) = r.sensor_elevation_rate_dps {
        env.call_method(
            &b,
            "sensorElevationRateDps",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }
    if let Some(v) = r.sensor_roll_rate_dps {
        env.call_method(
            &b,
            "sensorRollRateDps",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }
    if let Some(v) = r.mi_storage_percent_full {
        env.call_method(
            &b,
            "miStoragePercentFull",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }
    if let Some(v) = r.transmission_frequency_mhz {
        env.call_method(
            &b,
            "transmissionFrequencyMhz",
            BUILDER_SIG_DBL,
            &[JValue::Double(v)],
        )?;
    }
    if let Some(v) = r.zoom_percentage {
        env.call_method(&b, "zoomPercentage", BUILDER_SIG_DBL, &[JValue::Double(v)])?;
    }

    // --- WP-B Table B2: var-length int/enum fields (tags 110-139) ---
    // Tag 110 — timeAirborneS (u32 → long)
    if let Some(v) = r.time_airborne_s {
        env.call_method(
            &b,
            "timeAirborneS",
            BUILDER_SIG_LONG,
            &[JValue::Long(i64::from(v))],
        )?;
    }
    // Tag 111 — propulsionUnitSpeedRpm (u32 → long)
    if let Some(v) = r.propulsion_unit_speed_rpm {
        env.call_method(
            &b,
            "propulsionUnitSpeedRpm",
            BUILDER_SIG_LONG,
            &[JValue::Long(i64::from(v))],
        )?;
    }
    // Tag 123 — navsatsInView (u8 → int)
    if let Some(v) = r.navsats_in_view {
        env.call_method(
            &b,
            "navsatsInView",
            BUILDER_SIG_INT,
            &[JValue::Int(i32::from(v))],
        )?;
    }
    // Tag 124 — positioningMethodSource (u8 bitfield → int)
    if let Some(v) = r.positioning_method_source {
        env.call_method(
            &b,
            "positioningMethodSource",
            BUILDER_SIG_INT,
            &[JValue::Int(i32::from(v))],
        )?;
    }
    // Tag 125 — platformStatusCode (raw codepoint → int)
    if let Some(v) = r.platform_status {
        env.call_method(
            &b,
            "platformStatusCode",
            BUILDER_SIG_INT,
            &[JValue::Int(i32::from(platform_status_to_code(v)))],
        )?;
    }
    // Tag 126 — sensorControlModeCode (raw codepoint → int)
    if let Some(v) = r.sensor_control_mode {
        env.call_method(
            &b,
            "sensorControlModeCode",
            BUILDER_SIG_INT,
            &[JValue::Int(i32::from(sensor_control_mode_to_code(v)))],
        )?;
    }
    // Tag 131 — takeOffTimeUs (u64 → long; mirrors timestampUs's `as i64` cast)
    if let Some(v) = r.take_off_time_us {
        env.call_method(
            &b,
            "takeOffTimeUs",
            BUILDER_SIG_LONG,
            &[JValue::Long(v as i64)],
        )?;
    }
    // Tag 133 — miStorageCapacityGb (u32 → long)
    if let Some(v) = r.mi_storage_capacity_gb {
        env.call_method(
            &b,
            "miStorageCapacityGb",
            BUILDER_SIG_LONG,
            &[JValue::Long(i64::from(v))],
        )?;
    }
    // Tag 136 — leapSeconds (i32 → int)
    if let Some(v) = r.leap_seconds {
        env.call_method(&b, "leapSeconds", BUILDER_SIG_INT, &[JValue::Int(v)])?;
    }
    // Tag 137 — correctionOffsetUs (i64 → long)
    if let Some(v) = r.correction_offset_us {
        env.call_method(
            &b,
            "correctionOffsetUs",
            BUILDER_SIG_LONG,
            &[JValue::Long(v)],
        )?;
    }
    // Tag 139 — activePayloads (Vec<u8> → heap ByteBuffer)
    if let Some(ref bs) = r.active_payloads {
        let buf = wrap_heap_byte_buffer(env, bs).map_err(|()| jni::errors::Error::JavaException)?;
        env.call_method(
            &b,
            "activePayloads",
            BUILDER_SIG_BUF,
            &[JValue::Object(&buf)],
        )?;
    }

    // --- WP-A Table A4: named nested-set raw byte fields → heap ByteBuffer ---
    // Tag 73 — rvt
    if let Some(ref bs) = r.rvt {
        let buf = wrap_heap_byte_buffer(env, bs).map_err(|()| jni::errors::Error::JavaException)?;
        env.call_method(&b, "rvt", BUILDER_SIG_BUF, &[JValue::Object(&buf)])?;
    }
    // Tag 95 — sarMiLocalSet
    if let Some(ref bs) = r.sar_mi_local_set {
        let buf = wrap_heap_byte_buffer(env, bs).map_err(|()| jni::errors::Error::JavaException)?;
        env.call_method(
            &b,
            "sarMiLocalSet",
            BUILDER_SIG_BUF,
            &[JValue::Object(&buf)],
        )?;
    }
    // Tag 97 — rangeImageLocalSet
    if let Some(ref bs) = r.range_image_local_set {
        let buf = wrap_heap_byte_buffer(env, bs).map_err(|()| jni::errors::Error::JavaException)?;
        env.call_method(
            &b,
            "rangeImageLocalSet",
            BUILDER_SIG_BUF,
            &[JValue::Object(&buf)],
        )?;
    }
    // Tag 98 — geoRegistrationLocalSet
    if let Some(ref bs) = r.geo_registration_local_set {
        let buf = wrap_heap_byte_buffer(env, bs).map_err(|()| jni::errors::Error::JavaException)?;
        env.call_method(
            &b,
            "geoRegistrationLocalSet",
            BUILDER_SIG_BUF,
            &[JValue::Object(&buf)],
        )?;
    }
    // Tag 99 — compositeImagingLocalSet
    if let Some(ref bs) = r.composite_imaging_local_set {
        let buf = wrap_heap_byte_buffer(env, bs).map_err(|()| jni::errors::Error::JavaException)?;
        env.call_method(
            &b,
            "compositeImagingLocalSet",
            BUILDER_SIG_BUF,
            &[JValue::Object(&buf)],
        )?;
    }
    // Tag 100 — segmentLocalSet
    if let Some(ref bs) = r.segment_local_set {
        let buf = wrap_heap_byte_buffer(env, bs).map_err(|()| jni::errors::Error::JavaException)?;
        env.call_method(
            &b,
            "segmentLocalSet",
            BUILDER_SIG_BUF,
            &[JValue::Object(&buf)],
        )?;
    }
    // Tag 101 — amendLocalSet
    if let Some(ref bs) = r.amend_local_set {
        let buf = wrap_heap_byte_buffer(env, bs).map_err(|()| jni::errors::Error::JavaException)?;
        env.call_method(
            &b,
            "amendLocalSet",
            BUILDER_SIG_BUF,
            &[JValue::Object(&buf)],
        )?;
    }

    // --- WP-A Table A2: raw/simple scalar + string fields ---
    // Tag 39 — outsideAirTempC (Option<i8> → Integer; safe widening, no narrowing here)
    if let Some(v) = r.outside_air_temp_c {
        env.call_method(
            &b,
            "outsideAirTempC",
            BUILDER_SIG_INT,
            &[JValue::Int(i32::from(v))],
        )?;
    }
    // Tag 60 — weaponLoad (Option<u16> → Integer)
    if let Some(v) = r.weapon_load {
        env.call_method(
            &b,
            "weaponLoad",
            BUILDER_SIG_INT,
            &[JValue::Int(i32::from(v))],
        )?;
    }
    // Tag 61 — weaponFired (Option<u8> → Integer)
    if let Some(v) = r.weapon_fired {
        env.call_method(
            &b,
            "weaponFired",
            BUILDER_SIG_INT,
            &[JValue::Int(i32::from(v))],
        )?;
    }
    // Tag 62 — laserPrfCode (Option<u16> → Integer)
    if let Some(v) = r.laser_prf_code {
        env.call_method(
            &b,
            "laserPrfCode",
            BUILDER_SIG_INT,
            &[JValue::Int(i32::from(v))],
        )?;
    }
    // Tag 70 — alternatePlatformName (Option<String>)
    if let Some(ref v) = r.alternate_platform_name {
        let j = env.new_string(v)?;
        env.call_method(
            &b,
            "alternatePlatformName",
            BUILDER_SIG_STR,
            &[JValue::Object(&j)],
        )?;
    }
    // Tag 72 — eventStartTimeUs (Option<u64> → Long; mirrors timestampUs's `as i64` cast)
    if let Some(v) = r.event_start_time_us {
        env.call_method(
            &b,
            "eventStartTimeUs",
            BUILDER_SIG_LONG,
            &[JValue::Long(v as i64)],
        )?;
    }
    // Tag 106 — streamDesignator (Option<String>)
    if let Some(ref v) = r.stream_designator {
        let j = env.new_string(v)?;
        env.call_method(
            &b,
            "streamDesignator",
            BUILDER_SIG_STR,
            &[JValue::Object(&j)],
        )?;
    }
    // Tag 107 — operationalBase (Option<String>)
    if let Some(ref v) = r.operational_base {
        let j = env.new_string(v)?;
        env.call_method(
            &b,
            "operationalBase",
            BUILDER_SIG_STR,
            &[JValue::Object(&j)],
        )?;
    }
    // Tag 108 — broadcastSource (Option<String>)
    if let Some(ref v) = r.broadcast_source {
        let j = env.new_string(v)?;
        env.call_method(
            &b,
            "broadcastSource",
            BUILDER_SIG_STR,
            &[JValue::Object(&j)],
        )?;
    }
    // Tag 129 — targetId (Option<String>)
    if let Some(ref v) = r.target_id {
        let j = env.new_string(v)?;
        env.call_method(&b, "targetId", BUILDER_SIG_STR, &[JValue::Object(&j)])?;
    }
    // Tag 135 — communicationsMethod (Option<String>)
    if let Some(ref v) = r.communications_method {
        let j = env.new_string(v)?;
        env.call_method(
            &b,
            "communicationsMethod",
            BUILDER_SIG_STR,
            &[JValue::Object(&j)],
        )?;
    }

    // --- WP-A Table A3: coded enums (tags 34/63/77) → raw-codepoint Integer ---
    // Tag 34 — icingDetectedCode
    if let Some(v) = r.icing_detected {
        env.call_method(
            &b,
            "icingDetectedCode",
            BUILDER_SIG_INT,
            &[JValue::Int(i32::from(icing_detected_to_code(v)))],
        )?;
    }
    // Tag 63 — sensorFovNameCode
    if let Some(v) = r.sensor_fov_name {
        env.call_method(
            &b,
            "sensorFovNameCode",
            BUILDER_SIG_INT,
            &[JValue::Int(i32::from(sensor_fov_name_to_code(v)))],
        )?;
    }
    // Tag 77 — operationalModeCode
    if let Some(v) = r.operational_mode {
        env.call_method(
            &b,
            "operationalModeCode",
            BUILDER_SIG_INT,
            &[JValue::Int(i32::from(operational_mode_to_code(v)))],
        )?;
    }

    // --- fieldErrors — always set (even if empty) ---
    let fe_list = build_field_errors(env, &r.field_errors)?;
    env.call_method(
        &b,
        "fieldErrors",
        BUILDER_SIG_LIST,
        &[JValue::Object(&fe_list)],
    )?;

    // --- sentinelTags — always set (even if empty) ---
    let sentinel_list = build_long_list(env, r.sentinel_tags.iter().map(|&t| t as i64))?;
    env.call_method(
        &b,
        "sentinelTags",
        BUILDER_SIG_LIST,
        &[JValue::Object(&sentinel_list)],
    )?;

    // --- imapbSpecials: Vec<(u32, ImapbSpecial)> → List<ImapbSpecialEntry>, always set ---
    let specials_list = env.new_object("java/util/ArrayList", "()V", &[])?;
    for &(tag, special) in &r.imapb_specials {
        let (code, payload) = imapb_special_to_code(env, special)?;
        // 8 slots: covers the code String + entry object refs + JNI scratch per entry.
        env.with_local_frame(8, |env| -> jni::errors::Result<()> {
            let code_str = env.new_string(code)?;
            let entry = env.new_object(
                "org/tstrans/klv/ImapbSpecialEntry",
                "(ILjava/lang/String;J)V",
                &[
                    JValue::Int(tag as i32),
                    JValue::Object(&code_str),
                    JValue::Long(payload as i64),
                ],
            )?;
            env.call_method(
                &specials_list,
                "add",
                "(Ljava/lang/Object;)Z",
                &[JValue::Object(&entry)],
            )?;
            Ok(())
        })?;
    }
    env.call_method(
        &b,
        "imapbSpecials",
        BUILDER_SIG_LIST,
        &[JValue::Object(&specials_list)],
    )?;

    // --- unknown — always set (even if empty) ---
    let unk_list = build_unknown_list(env, &r.unknown)?;
    env.call_method(
        &b,
        "unknown",
        BUILDER_SIG_LIST,
        &[JValue::Object(&unk_list)],
    )?;

    // build() → UasDatalinkLs
    let built = env
        .call_method(&b, "build", "()Lorg/tstrans/klv/UasDatalinkLs;", &[])?
        .l()?;
    Ok(built.into_raw())
}

// -----------------------------------------------------------------------
// Java → Rust reader (encode path)
// -----------------------------------------------------------------------

/// Read all fields from a Java `UasDatalinkLs` record into a Rust `UasDatalinkLs`.
/// Mirrors tst-py's `py_to_uas_datalink_ls` including:
/// - 16-byte UL validation (raises RuntimeException on wrong length)
/// - `is_st0601_typed_tag` collision-drop on `unknown`
/// - `field_errors` not round-tripped (decoder-only diagnostic)
#[allow(clippy::field_reassign_with_default)]
fn read_uas_datalink(
    env: &mut JNIEnv<'_>,
    rec: &JObject<'_>,
) -> jni::errors::Result<UasDatalinkLs> {
    let mut r = UasDatalinkLs::default();

    // --- universal_label: ByteBuffer → UniversalLabel([u8;16]) ---
    // Use read_byte_buffer to honour position/limit and support direct buffers.
    {
        let bb_obj = env
            .call_method(rec, "universalLabel", "()Ljava/nio/ByteBuffer;", &[])?
            .l()?;
        let ul_bytes = read_byte_buffer(env, &bb_obj)?;
        if ul_bytes.len() != 16 {
            let _ = env.throw_new(
                "java/lang/IllegalArgumentException",
                format!("universalLabel must be 16 bytes, got {}", ul_bytes.len()),
            );
            return Err(jni::errors::Error::JavaException);
        }
        let mut ul = [0u8; 16];
        ul.copy_from_slice(&ul_bytes);
        r.universal_label = UniversalLabel(ul);
    }

    // --- declared_version: int → u8 (non-optional, primitive) ---
    {
        let v = env.call_method(rec, "declaredVersion", "()I", &[])?.i()?;
        r.declared_version = checked_u8(env, i64::from(v), "declaredVersion")?;
    }

    // --- Identity: nullable String fields ---
    r.mission_id = read_nullable_string(env, rec, "missionId")?;
    r.platform_tail_number = read_nullable_string(env, rec, "platformTailNumber")?;
    r.platform_designation = read_nullable_string(env, rec, "platformDesignation")?;
    r.image_source_sensor = read_nullable_string(env, rec, "imageSourceSensor")?;
    r.image_coordinate_system = read_nullable_string(env, rec, "imageCoordinateSystem")?;
    r.platform_call_sign = read_nullable_string(env, rec, "platformCallSign")?;

    // --- uasLsVersion: nullable Integer → Option<u8> ---
    if let Some(v) = read_nullable_int(env, rec, "uasLsVersion")? {
        r.uas_ls_version = Some(checked_u8(env, i64::from(v), "uasLsVersion")?);
    }

    // --- timestampUs: nullable Long → Option<u64> ---
    if let Some(v) = read_nullable_long(env, rec, "timestampUs")? {
        r.timestamp_us = Some(v as u64);
    }

    // --- Platform state: nullable Double → Option<f64> ---
    if let Some(v) = read_nullable_double(env, rec, "platformHeadingDeg")? {
        r.platform_heading_deg = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "platformPitchDeg")? {
        r.platform_pitch_deg = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "platformRollDeg")? {
        r.platform_roll_deg = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "platformTrueAirspeed")? {
        r.platform_true_airspeed = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "platformIndicatedAirspeed")? {
        r.platform_indicated_airspeed = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "platformPitchFullDeg")? {
        r.platform_pitch_full_deg = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "platformRollFullDeg")? {
        r.platform_roll_full_deg = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "platformAngleOfAttackDeg")? {
        r.platform_angle_of_attack_deg = Some(v);
    }

    // --- Sensor pose & position ---
    if let Some(v) = read_nullable_double(env, rec, "sensorLatDeg")? {
        r.sensor_lat_deg = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "sensorLonDeg")? {
        r.sensor_lon_deg = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "sensorAltM")? {
        r.sensor_alt_m = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "sensorEllipsoidHeightM")? {
        r.sensor_ellipsoid_height_m = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "sensorHfovDeg")? {
        r.sensor_hfov_deg = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "sensorVfovDeg")? {
        r.sensor_vfov_deg = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "sensorRelAzDeg")? {
        r.sensor_rel_az_deg = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "sensorRelElDeg")? {
        r.sensor_rel_el_deg = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "sensorRelRollDeg")? {
        r.sensor_rel_roll_deg = Some(v);
    }

    // --- Ranging & frame center ---
    if let Some(v) = read_nullable_double(env, rec, "slantRangeM")? {
        r.slant_range_m = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "targetWidthM")? {
        r.target_width_m = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "frameCenterLatDeg")? {
        r.frame_center_lat_deg = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "frameCenterLonDeg")? {
        r.frame_center_lon_deg = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "frameCenterElevM")? {
        r.frame_center_elev_m = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "frameCenterEllipsoidHeightM")? {
        r.frame_center_ellipsoid_height_m = Some(v);
    }

    // --- Image corner offsets (tags 26–33) ---
    if let Some(v) = read_nullable_double(env, rec, "cornerLatOffsetP1Deg")? {
        r.corner_lat_offset_p1_deg = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "cornerLonOffsetP1Deg")? {
        r.corner_lon_offset_p1_deg = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "cornerLatOffsetP2Deg")? {
        r.corner_lat_offset_p2_deg = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "cornerLonOffsetP2Deg")? {
        r.corner_lon_offset_p2_deg = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "cornerLatOffsetP3Deg")? {
        r.corner_lat_offset_p3_deg = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "cornerLonOffsetP3Deg")? {
        r.corner_lon_offset_p3_deg = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "cornerLatOffsetP4Deg")? {
        r.corner_lat_offset_p4_deg = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "cornerLonOffsetP4Deg")? {
        r.corner_lon_offset_p4_deg = Some(v);
    }

    // --- Image corners full lat/lon (tags 82–89) ---
    if let Some(v) = read_nullable_double(env, rec, "cornerLatP1Deg")? {
        r.corner_lat_p1_deg = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "cornerLonP1Deg")? {
        r.corner_lon_p1_deg = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "cornerLatP2Deg")? {
        r.corner_lat_p2_deg = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "cornerLonP2Deg")? {
        r.corner_lon_p2_deg = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "cornerLatP3Deg")? {
        r.corner_lat_p3_deg = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "cornerLonP3Deg")? {
        r.corner_lon_p3_deg = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "cornerLatP4Deg")? {
        r.corner_lat_p4_deg = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "cornerLonP4Deg")? {
        r.corner_lon_p4_deg = Some(v);
    }

    // --- Misc ---
    if let Some(v) = read_nullable_int(env, rec, "genericFlagData")? {
        r.generic_flag_data = Some(checked_u8(env, i64::from(v), "genericFlagData")?);
    }
    r.security_local_set = read_nullable_byte_buffer(env, rec, "securityLocalSet")?;
    r.vmti = read_nullable_byte_buffer(env, rec, "vmti")?;

    // --- miisCoreId: nullable byte[] → Option<Vec<u8>> ---
    {
        let arr_val = env.call_method(rec, "miisCoreId", "()[B", &[])?.l()?;
        if !arr_val.is_null() {
            let arr: jni::objects::JByteArray<'_> = arr_val.into();
            r.miis_core_id = Some(env.convert_byte_array(&arr)?);
        }
    }

    // --- WP-A Table A1: ranged f64 fields (tags 35-93 subset) ---
    if let Some(v) = read_nullable_double(env, rec, "targetLocationLatDeg")? {
        r.target_location_lat_deg = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "targetLocationLonDeg")? {
        r.target_location_lon_deg = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "targetLocationElevM")? {
        r.target_location_elev_m = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "targetTrackGateWidthPx")? {
        r.target_track_gate_width_px = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "targetTrackGateHeightPx")? {
        r.target_track_gate_height_px = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "targetErrorCe90M")? {
        r.target_error_ce90_m = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "targetErrorLe90M")? {
        r.target_error_le90_m = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "windDirectionDeg")? {
        r.wind_direction_deg = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "windSpeed")? {
        r.wind_speed = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "staticPressureMbar")? {
        r.static_pressure_mbar = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "densityAltitudeM")? {
        r.density_altitude_m = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "differentialPressureMbar")? {
        r.differential_pressure_mbar = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "airfieldBarometricPressureMbar")? {
        r.airfield_barometric_pressure_mbar = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "airfieldElevationM")? {
        r.airfield_elevation_m = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "relativeHumidityPct")? {
        r.relative_humidity_pct = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "platformVerticalSpeed")? {
        r.platform_vertical_speed = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "platformSideslipDeg")? {
        r.platform_sideslip_deg = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "platformGroundSpeed")? {
        r.platform_ground_speed = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "groundRangeM")? {
        r.ground_range_m = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "platformFuelRemainingKg")? {
        r.platform_fuel_remaining_kg = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "platformMagneticHeadingDeg")? {
        r.platform_magnetic_heading_deg = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "platformAngleOfAttackFullDeg")? {
        r.platform_angle_of_attack_full_deg = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "platformSideslipFullDeg")? {
        r.platform_sideslip_full_deg = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "alternatePlatformLatDeg")? {
        r.alternate_platform_lat_deg = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "alternatePlatformLonDeg")? {
        r.alternate_platform_lon_deg = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "alternatePlatformAltM")? {
        r.alternate_platform_alt_m = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "alternatePlatformHeadingDeg")? {
        r.alternate_platform_heading_deg = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "alternatePlatformEllipsoidHeightM")? {
        r.alternate_platform_ellipsoid_height_m = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "sensorNorthVelocity")? {
        r.sensor_north_velocity = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "sensorEastVelocity")? {
        r.sensor_east_velocity = Some(v);
    }

    // --- WP-B Table B1: IMAPB f64 fields ---
    if let Some(v) = read_nullable_double(env, rec, "targetWidthExtendedM")? {
        r.target_width_extended_m = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "densityAltitudeExtendedM")? {
        r.density_altitude_extended_m = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "sensorEllipsoidHeightExtendedM")? {
        r.sensor_ellipsoid_height_extended_m = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "alternatePlatformEllipsoidHeightExtendedM")? {
        r.alternate_platform_ellipsoid_height_extended_m = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "rangeToRecoveryKm")? {
        r.range_to_recovery_km = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "platformCourseAngleDeg")? {
        r.platform_course_angle_deg = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "altitudeAglM")? {
        r.altitude_agl_m = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "radarAltimeterM")? {
        r.radar_altimeter_m = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "sensorAzimuthRateDps")? {
        r.sensor_azimuth_rate_dps = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "sensorElevationRateDps")? {
        r.sensor_elevation_rate_dps = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "sensorRollRateDps")? {
        r.sensor_roll_rate_dps = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "miStoragePercentFull")? {
        r.mi_storage_percent_full = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "transmissionFrequencyMhz")? {
        r.transmission_frequency_mhz = Some(v);
    }
    if let Some(v) = read_nullable_double(env, rec, "zoomPercentage")? {
        r.zoom_percentage = Some(v);
    }

    // --- WP-B Table B2: var-length int/enum fields ---
    // Tag 110 — timeAirborneS: nullable Long → Option<u32>
    if let Some(v) = read_nullable_long(env, rec, "timeAirborneS")? {
        r.time_airborne_s = Some(checked_u32(env, v, "timeAirborneS")?);
    }
    // Tag 111 — propulsionUnitSpeedRpm: nullable Long → Option<u32>
    if let Some(v) = read_nullable_long(env, rec, "propulsionUnitSpeedRpm")? {
        r.propulsion_unit_speed_rpm = Some(checked_u32(env, v, "propulsionUnitSpeedRpm")?);
    }
    // Tag 123 — navsatsInView: nullable Integer → Option<u8>
    if let Some(v) = read_nullable_int(env, rec, "navsatsInView")? {
        r.navsats_in_view = Some(checked_u8(env, i64::from(v), "navsatsInView")?);
    }
    // Tag 124 — positioningMethodSource: nullable Integer → Option<u8>
    if let Some(v) = read_nullable_int(env, rec, "positioningMethodSource")? {
        r.positioning_method_source =
            Some(checked_u8(env, i64::from(v), "positioningMethodSource")?);
    }
    // Tag 125 — platformStatusCode: nullable Integer raw code → Option<PlatformStatus>
    if let Some(v) = read_nullable_int(env, rec, "platformStatusCode")? {
        let c = checked_u8(env, i64::from(v), "platformStatusCode")?;
        r.platform_status = Some(platform_status_from_code(c));
    }
    // Tag 126 — sensorControlModeCode: nullable Integer raw code → Option<SensorControlMode>
    if let Some(v) = read_nullable_int(env, rec, "sensorControlModeCode")? {
        let c = checked_u8(env, i64::from(v), "sensorControlModeCode")?;
        r.sensor_control_mode = Some(sensor_control_mode_from_code(c));
    }
    // Tag 131 — takeOffTimeUs: nullable Long → Option<u64> (mirrors timestampUs's `as u64` cast)
    if let Some(v) = read_nullable_long(env, rec, "takeOffTimeUs")? {
        r.take_off_time_us = Some(v as u64);
    }
    // Tag 133 — miStorageCapacityGb: nullable Long → Option<u32>
    if let Some(v) = read_nullable_long(env, rec, "miStorageCapacityGb")? {
        r.mi_storage_capacity_gb = Some(checked_u32(env, v, "miStorageCapacityGb")?);
    }
    // Tag 136 — leapSeconds: nullable Integer → Option<i32> (Java int == Rust i32, no narrowing)
    if let Some(v) = read_nullable_int(env, rec, "leapSeconds")? {
        r.leap_seconds = Some(v);
    }
    // Tag 137 — correctionOffsetUs: nullable Long → Option<i64> (Java long == Rust i64, no narrowing)
    if let Some(v) = read_nullable_long(env, rec, "correctionOffsetUs")? {
        r.correction_offset_us = Some(v);
    }
    // Tag 139 — activePayloads: nullable ByteBuffer → Option<Vec<u8>>
    r.active_payloads = read_nullable_byte_buffer(env, rec, "activePayloads")?;

    // --- WP-A Table A4: named nested-set raw byte fields ---
    r.rvt = read_nullable_byte_buffer(env, rec, "rvt")?;
    r.sar_mi_local_set = read_nullable_byte_buffer(env, rec, "sarMiLocalSet")?;
    r.range_image_local_set = read_nullable_byte_buffer(env, rec, "rangeImageLocalSet")?;
    r.geo_registration_local_set = read_nullable_byte_buffer(env, rec, "geoRegistrationLocalSet")?;
    r.composite_imaging_local_set =
        read_nullable_byte_buffer(env, rec, "compositeImagingLocalSet")?;
    r.segment_local_set = read_nullable_byte_buffer(env, rec, "segmentLocalSet")?;
    r.amend_local_set = read_nullable_byte_buffer(env, rec, "amendLocalSet")?;

    // --- WP-A Table A2: raw/simple scalar + string fields ---
    // Tag 39 — outsideAirTempC: nullable Integer → Option<i8>
    if let Some(v) = read_nullable_int(env, rec, "outsideAirTempC")? {
        r.outside_air_temp_c = Some(checked_i8(env, i64::from(v), "outsideAirTempC")?);
    }
    // Tag 60 — weaponLoad: nullable Integer → Option<u16>
    if let Some(v) = read_nullable_int(env, rec, "weaponLoad")? {
        r.weapon_load = Some(checked_u16(env, i64::from(v), "weaponLoad")?);
    }
    // Tag 61 — weaponFired: nullable Integer → Option<u8>
    if let Some(v) = read_nullable_int(env, rec, "weaponFired")? {
        r.weapon_fired = Some(checked_u8(env, i64::from(v), "weaponFired")?);
    }
    // Tag 62 — laserPrfCode: nullable Integer → Option<u16>
    if let Some(v) = read_nullable_int(env, rec, "laserPrfCode")? {
        r.laser_prf_code = Some(checked_u16(env, i64::from(v), "laserPrfCode")?);
    }
    r.alternate_platform_name = read_nullable_string(env, rec, "alternatePlatformName")?;
    // Tag 72 — eventStartTimeUs: nullable Long → Option<u64> (mirrors timestampUs's `as u64` cast)
    if let Some(v) = read_nullable_long(env, rec, "eventStartTimeUs")? {
        r.event_start_time_us = Some(v as u64);
    }
    r.stream_designator = read_nullable_string(env, rec, "streamDesignator")?;
    r.operational_base = read_nullable_string(env, rec, "operationalBase")?;
    r.broadcast_source = read_nullable_string(env, rec, "broadcastSource")?;
    r.target_id = read_nullable_string(env, rec, "targetId")?;
    r.communications_method = read_nullable_string(env, rec, "communicationsMethod")?;

    // --- WP-A Table A3: coded enums (tags 34/63/77) — nullable Integer raw code ---
    if let Some(v) = read_nullable_int(env, rec, "icingDetectedCode")? {
        let c = checked_u8(env, i64::from(v), "icingDetectedCode")?;
        r.icing_detected = Some(icing_detected_from_code(c));
    }
    if let Some(v) = read_nullable_int(env, rec, "sensorFovNameCode")? {
        let c = checked_u8(env, i64::from(v), "sensorFovNameCode")?;
        r.sensor_fov_name = Some(sensor_fov_name_from_code(c));
    }
    if let Some(v) = read_nullable_int(env, rec, "operationalModeCode")? {
        let c = checked_u8(env, i64::from(v), "operationalModeCode")?;
        r.operational_mode = Some(operational_mode_from_code(c));
    }

    // --- unknown: List<KlvUnknownField> with is_st0601_typed_tag collision-drop ---
    {
        let unk_obj = env
            .call_method(rec, "unknown", "()Ljava/util/List;", &[])?
            .l()?;
        r.unknown = read_unknown_list(env, &unk_obj, is_st0601_typed_tag)?;
    }

    // --- sentinelTags: List<Long> → Vec<u32> ---
    {
        let st_obj = env
            .call_method(rec, "sentinelTags", "()Ljava/util/List;", &[])?
            .l()?;
        let size = env.call_method(&st_obj, "size", "()I", &[])?.i()?;
        for i in 0..size {
            let item = env
                .call_method(&st_obj, "get", "(I)Ljava/lang/Object;", &[JValue::Int(i)])?
                .l()?;
            let v = env.call_method(&item, "longValue", "()J", &[])?.j()?;
            match u32::try_from(v) {
                Ok(tag) => r.sentinel_tags.push(tag),
                Err(_) => {
                    let _ = env.throw_new(
                        "java/lang/IllegalArgumentException",
                        format!("sentinelTags entry out of u32 range: {v}"),
                    );
                    return Err(jni::errors::Error::JavaException);
                }
            }
        }
    }

    // --- imapbSpecials: List<ImapbSpecialEntry> → Vec<(u32, ImapbSpecial)> ---
    {
        let is_obj = env
            .call_method(rec, "imapbSpecials", "()Ljava/util/List;", &[])?
            .l()?;
        let size = env.call_method(&is_obj, "size", "()I", &[])?.i()?;
        for i in 0..size {
            let item = env
                .call_method(&is_obj, "get", "(I)Ljava/lang/Object;", &[JValue::Int(i)])?
                .l()?;
            let tag_i = env.call_method(&item, "tag", "()I", &[])?.i()?;
            let tag = match u32::try_from(tag_i) {
                Ok(t) => t,
                Err(_) => {
                    let _ = env.throw_new(
                        "java/lang/IllegalArgumentException",
                        format!("imapbSpecials entry tag out of u32 range: {tag_i}"),
                    );
                    return Err(jni::errors::Error::JavaException);
                }
            };
            let code_obj = env
                .call_method(&item, "code", "()Ljava/lang/String;", &[])?
                .l()?;
            let j_str: &jni::objects::JString = (&code_obj).into();
            let code: String = env.get_string(j_str).map(Into::into)?;
            let payload_j = env.call_method(&item, "payload", "()J", &[])?.j()?;
            let special = imapb_special_from_code(env, &code, payload_j as u64)?;
            r.imapb_specials.push((tag, special));
        }
    }

    // field_errors is a decoder-only diagnostic; not round-tripped (mirrors tst-py).
    Ok(r)
}

/// Thin public wrapper so `st1204::validateMismmsNative` can call the shared
/// reader without duplicating it. All the real logic lives in `read_uas_datalink`.
pub fn read_uas_datalink_for_validate(
    env: &mut JNIEnv<'_>,
    rec: &JObject<'_>,
) -> jni::errors::Result<UasDatalinkLs> {
    read_uas_datalink(env, rec)
}
