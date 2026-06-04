package org.tstrans;

/**
 * Version information for the tstrans JVM binding.
 *
 * <p>Bootstrap surface: a single native method proving the JNI pipeline. The
 * full {@code org.tstrans.*} surface lands in the surface-port wave.
 */
public final class Version {
    private Version() {}

    static {
        NativeLoader.load();
    }

    /** @return the native (Rust) workspace crate version, e.g. {@code "0.1.0"}. */
    public static native String versionString();
}
