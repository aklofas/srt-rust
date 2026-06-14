//! JNI surface for ST 0601 UAS Datalink LS decode/encode.
//!
//! `nDecodeUasDatalink(byte[], boolean strict, boolean compliance) -> UasDatalinkLs` —
//! dispatches: `compliance=true` → `decode_strict_compliance`; else `strict=true` →
//! `decode_strict`; else `decode` (lenient). Builds the Java `UasDatalinkLs` via its
//! public mutable `Builder` (the Builder-marshalling pattern from Tasks 2–3).
//!
//! `nEncodeUasDatalink(UasDatalinkLs) -> byte[]` — reads all fields via accessor
//! `call_method`s, builds a Rust `UasDatalinkLs`, calls `encode_to_vec`. Mirrors
//! tst-py's `py_to_uas_datalink_ls` including 16-byte UL validation and the
//! `is_st0601_typed_tag` collision-drop.
//!
//! `nEncodeUasDatalinkStrictCompliance(UasDatalinkLs) -> byte[]` — same read path,
//! calls `encode_strict_compliance` instead.
//!
//! ### JNI local-ref capacity (CRITICAL for 80-field set)
//!
//! `build_uas_datalink` calls `env.ensure_local_capacity(128)` at the top.
//! With ~50 String fields + ~30 Double/Long/ByteBuffer fields + builder + lists +
//! JNI scratch, 128 slots safely covers the worst-case fully-populated record.
//! Skipping this call WILL crash the JVM for records with many populated fields.

use jni::JNIEnv;
use jni::objects::{JByteArray, JClass, JObject, JValue};
use jni::sys::jobject;
use tst_core::klv::st0601::{
    UasDatalinkLs, decode as decode_lenient, decode_strict, decode_strict_compliance,
    encode_strict_compliance, encode_to_vec,
};
use tst_core::klv::universal_label::UniversalLabel;

use crate::error::{map_klv_decode_error, map_klv_encode_error};
use crate::jutil::{
    build_field_errors, build_unknown_list, checked_u8, read_byte_buffer,
    read_nullable_byte_buffer, read_nullable_double, read_nullable_int, read_nullable_long,
    read_nullable_string, read_unknown_list, wrap_heap_byte_buffer,
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

/// ST 0601 typed tags: 1, 2, 65, 5..=91.
/// Mirrors tst-py's `is_st0601_typed_tag`.
fn is_st0601_typed_tag(tag: u32) -> bool {
    matches!(tag, 1 | 2 | 65 | 5..=91)
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

/// `org.tstrans.klv.Klv.nEncodeUasDatalink(UasDatalinkLs) -> byte[]`
///
/// Reads all fields from the Java `UasDatalinkLs` record, builds a Rust
/// `UasDatalinkLs`, calls `encode_to_vec`. Returns the full wire bytes
/// `[UL:16][BER length][body][Tag1 checksum]`. Mirrors tst-py's
/// `encode_uas_datalink`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_klv_Klv_nEncodeUasDatalink<'local>(
    mut env: JNIEnv<'local>,
    _c: JClass<'local>,
    record: JObject<'local>,
) -> jobject {
    crate::panic::jni_catch(
        &mut env,
        std::ptr::null_mut(),
        |env| match read_uas_datalink(env, &record) {
            Ok(rust_rec) => match encode_to_vec(&rust_rec) {
                Ok(bytes) => match env.byte_array_from_slice(&bytes) {
                    Ok(arr) => arr.into_raw(),
                    Err(e) => {
                        let _ = env.throw_new(
                            "java/lang/RuntimeException",
                            format!("nEncodeUasDatalink: byte_array_from_slice failed: {e}"),
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
                        format!("nEncodeUasDatalink: field read failed: {e}"),
                    );
                }
                JObject::null().into_raw()
            }
        },
    )
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
// Rust → Java builder (decode path)
// -----------------------------------------------------------------------

/// Build a `org.tstrans.klv.UasDatalinkLs` Java record from a Rust `UasDatalinkLs`
/// via the public mutable `Builder`. Mirrors `convert_uas_datalink_ls` in tst-py.
///
/// ### Local-ref capacity (MANDATORY)
///
/// Calls `env.ensure_local_capacity(128)` at the top. With ~80 fields (50+ Strings,
/// 30 boxed scalars, 2 ByteBuffers, builder + lists), the default ~16-slot JNI local
/// table is completely inadequate. 128 slots is the minimum safe value for a fully
/// populated ST 0601 record.
fn build_uas_datalink(env: &mut JNIEnv<'_>, r: &UasDatalinkLs) -> jni::errors::Result<jobject> {
    // CRITICAL: must be called before any new_string / new_object below.
    // 128 slots covers ~80 fields + builder + lists + JNI scratch.
    env.ensure_local_capacity(128)?;

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

    // --- fieldErrors — always set (even if empty) ---
    let fe_list = build_field_errors(env, &r.field_errors)?;
    env.call_method(
        &b,
        "fieldErrors",
        BUILDER_SIG_LIST,
        &[JValue::Object(&fe_list)],
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

    // --- unknown: List<KlvUnknownField> with is_st0601_typed_tag collision-drop ---
    {
        let unk_obj = env
            .call_method(rec, "unknown", "()Ljava/util/List;", &[])?
            .l()?;
        r.unknown = read_unknown_list(env, &unk_obj, is_st0601_typed_tag)?;
    }

    // field_errors is a decoder-only diagnostic; not round-tripped (mirrors tst-py).
    Ok(r)
}
