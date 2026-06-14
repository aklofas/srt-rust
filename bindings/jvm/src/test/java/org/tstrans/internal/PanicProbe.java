package org.tstrans.internal;

/**
 * Test-only JNI hook (never shipped). Binds to the Rust symbol
 * {@code Java_org_tstrans_internal_PanicProbe_nForcePanic}, compiled only under
 * the {@code jni-test-hooks} cargo feature. {@link #nForcePanic()} panics inside
 * the native body; with {@code jni_catch} in place the panic surfaces as a
 * {@link RuntimeException} instead of aborting the JVM.
 */
public final class PanicProbe {
    private PanicProbe() {}

    // Mirror the sibling test classes' load mechanism: the static block runs
    // NativeLoader.load() (idempotent) so the cdylib is resolved before
    // nForcePanic() is invoked. Fully qualified because this class lives in the
    // org.tstrans.internal package.
    static { org.tstrans.NativeLoader.load(); }

    public static native long nForcePanic();
}
