//! JVM (JNI) bindings — bootstrap surface.
//!
//! This crate currently exports exactly one function to prove the
//! cargo -> cdylib -> Gradle -> Java -> JNI build pipeline end to end. The full
//! `org.tstrans.*` surface (mirroring tst-py) lands in the step-2 surface port.

mod error;

use jni::JNIEnv;
use jni::objects::JClass;
use jni::sys::jstring;

/// `org.tstrans.Version.versionString()` — returns the Rust workspace crate
/// version (e.g. "0.1.0") as a Java string, proving a value crosses the JNI
/// boundary from Rust to the JVM.
///
/// `#[unsafe(no_mangle)]` (edition 2024 spelling) keeps the symbol name exactly
/// `Java_org_tstrans_Version_versionString` so the JVM's native-method linker
/// resolves it.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_Version_versionString<'local>(
    env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> jstring {
    env.new_string(env!("CARGO_PKG_VERSION"))
        .expect("failed to allocate Java string")
        .into_raw()
}
