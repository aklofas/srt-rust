//! JVM (JNI) bindings.
//!
//! Exports the bootstrap `org.tstrans.Version.versionString()` plus the Wave 1
//! mpegts-demux keystone (`org.tstrans.mpegts.Demuxer` + `DemuxEvent`); see
//! `mod mpegts`. The remaining `org.tstrans.*` modules (mirroring tst-py) land
//! in the follow-on surface-port waves.

mod error;
mod jutil;
mod klv;
mod mpegts;

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
