//! JNI surface for `org.tstrans.klv` — typed KLV decode/encode for
//! ST 0601 / 0102 / 0605 / 0903. Per-set entry points live in submodules
//! (filled across Tasks 1–4); this module houses the test-only forced-throw
//! helpers that let `KlvErrorModelTest` exercise the error-mapping wiring
//! before the real decode/encode entry points exist.

pub mod st0102;
pub mod st0601;
pub mod st0605;
pub mod st0903;

use jni::JNIEnv;
use jni::objects::{JClass, JString};

use crate::error::{throw_klv_decode, throw_klv_encode};

/// Test-only: `org.tstrans.klv.Klv.nRaiseDecodeForTest(kind)`.
/// Throws a `KlvDecodeException` with the given Kind name, allowing
/// `KlvErrorModelTest` to verify the full error-mapping path.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_klv_Klv_nRaiseDecodeForTest<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    kind: JString<'local>,
) {
    let k: String = env.get_string(&kind).map(Into::into).unwrap_or_default();
    throw_klv_decode(&mut env, &k, "forced decode error for test");
}

/// Test-only: `org.tstrans.klv.Klv.nRaiseEncodeForTest(kind)`.
/// Throws a `KlvEncodeException` with the given Kind name (no tag), allowing
/// `KlvErrorModelTest` to verify the full error-mapping path.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_klv_Klv_nRaiseEncodeForTest<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    kind: JString<'local>,
) {
    let k: String = env.get_string(&kind).map(Into::into).unwrap_or_default();
    throw_klv_encode(&mut env, &k, None, "forced encode error for test");
}
