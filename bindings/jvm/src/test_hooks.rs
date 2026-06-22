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

// ---------------------------------------------------------------------------
// Handle-aware probe: exercises `HandleRegistry::with_poisoning` end-to-end
// through a real JNI call. A registry-backed `u64` counter acts as the
// resource; the mutating native panics mid-mutation to prove the handle is
// poisoned and later ops throw `IllegalStateException`.
// ---------------------------------------------------------------------------

mod handle_probe {
    use crate::handle::HandleRegistry;
    use crate::panic::jni_catch;
    use jni::JNIEnv;
    use jni::objects::JClass;
    use jni::sys::{jboolean, jlong};
    use std::sync::LazyLock;

    static PROBE_REGISTRY: LazyLock<HandleRegistry<u64>> = LazyLock::new(HandleRegistry::new);

    /// `org.tstrans.internal.PanicProbe.nOpenHandle()` — open a registry-backed
    /// probe handle; returns the opaque non-zero key.
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_org_tstrans_internal_PanicProbe_nOpenHandle<'local>(
        mut env: JNIEnv<'local>,
        _class: JClass<'local>,
    ) -> jlong {
        jni_catch(&mut env, 0, |_env| PROBE_REGISTRY.insert(0) as jlong)
    }

    /// `org.tstrans.internal.PanicProbe.nMutateMaybePanic(long, boolean)` — run
    /// a mutation through [`HandleRegistry::with_poisoning`]. When `doPanic` is
    /// `true`, panics mid-mutation; the outer `jni_catch` surfaces the panic as a
    /// `RuntimeException`. A subsequent call on the same (now-poisoned) handle
    /// leases `None` and throws `IllegalStateException`.
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_org_tstrans_internal_PanicProbe_nMutateMaybePanic<'local>(
        mut env: JNIEnv<'local>,
        _class: JClass<'local>,
        handle: jlong,
        do_panic: jboolean,
    ) {
        jni_catch(&mut env, (), |env| {
            let panic = do_panic != 0;
            let outcome = PROBE_REGISTRY.with_poisoning(handle as u64, |v| {
                *v = v.wrapping_add(1); // begin a mutation
                if panic {
                    panic!("probe: torn mutation");
                }
            });
            if outcome.is_none() {
                let _ = env.throw_new(
                    "java/lang/IllegalStateException",
                    "probe handle is closed or poisoned",
                );
            }
        })
    }

    /// `org.tstrans.internal.PanicProbe.nCloseHandle(long)` — close the probe
    /// handle; idempotent, safe even on a poisoned handle.
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_org_tstrans_internal_PanicProbe_nCloseHandle<'local>(
        mut env: JNIEnv<'local>,
        _class: JClass<'local>,
        handle: jlong,
    ) {
        jni_catch(&mut env, (), |_env| {
            PROBE_REGISTRY.close(handle as u64);
        })
    }
}
