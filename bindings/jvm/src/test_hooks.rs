//! Test-only JNI hooks, compiled solely under `feature = "jni-test-hooks"`.
//! The published cdylib is built WITHOUT this feature (CI rebuilds probe-free
//! before staging the shipped lib — see `.github/workflows/jvm-jar.yml`), so
//! these symbols never reach Maven Central.

use jni::JNIEnv;
use jni::objects::JClass;
use jni::sys::jlong;

/// `org.tstrans.internal.PanicProbe.nForcePanic()` — deliberately panics inside
/// [`crate::panic::jni_catch`] to prove the panic becomes a thrown
/// `RuntimeException` rather than unwinding across the JNI boundary.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_internal_PanicProbe_nForcePanic<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> jlong {
    crate::panic::jni_catch(&mut env, 0, |_env| {
        panic!("intentional panic from PanicProbe.nForcePanic (jni-test-hooks)");
    })
}
