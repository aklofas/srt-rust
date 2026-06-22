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

    // --- Handle-aware probe (JNI-01b) ---------------------------------------
    /** Open a registry-backed probe handle; returns a non-zero opaque key. */
    public static native long nOpenHandle();
    /**
     * Mutate the handle through {@code with_poisoning}. When {@code doPanic}
     * is {@code true}, panics mid-mutation, surfacing as {@link RuntimeException}.
     * A subsequent call on the (now-poisoned) handle throws
     * {@link IllegalStateException}.
     */
    public static native void nMutateMaybePanic(long handle, boolean doPanic);
    /** Close the probe handle; idempotent and safe on a poisoned handle. */
    public static native void nCloseHandle(long handle);

    // --- RTSP-surface panic-routing probes (JNI-01 completion) --------------
    /**
     * Force a panic through the REAL {@code REGISTRY_MOUNT.with_poisoning} on a
     * leased {@code MountHandle} (exactly as a real {@code pushVideo}/{@code flush}
     * would route), surfacing as {@link RuntimeException}. After this returns the
     * mount entry is poisoned: a later real mount op throws
     * {@link IllegalStateException}. Proves the RTSP mount mutators are wired to
     * the poisoning path.
     */
    public static native void nForcePanicThroughMount(long mountHandle);
    /**
     * Force a panic through the REAL {@code REGISTRY_SERVER.with_poisoning} on a
     * leased {@code RtspServer} (as a real {@code stop}/{@code addUnicastMount}
     * would route), surfacing as {@link RuntimeException} and poisoning the server
     * entry. Proves the RTSP server mutators are wired to the poisoning path.
     */
    public static native void nForcePanicThroughServer(long serverHandle);
}
