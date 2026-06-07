package org.tstrans;

import static org.junit.jupiter.api.Assertions.assertEquals;

import org.junit.jupiter.api.Test;

/**
 * Locks the JAR resource layout the fat-JAR assembly depends on. The triple
 * here MUST match the directory names the CI assemble job stages into, so the
 * loader finds the lib at runtime. Apple Silicon resolves to macos-aarch64
 * (arch normalized arm64 -> aarch64), and Windows resolves to libtstjni.dll.
 */
class NativeLoaderTest {

    @Test
    void linuxX86() {
        assertEquals("/native/linux-x86_64/libtstjni.so",
                NativeLoader.resourcePath("Linux", "amd64"));
        assertEquals("/native/linux-x86_64/libtstjni.so",
                NativeLoader.resourcePath("Linux", "x86_64"));
    }

    @Test
    void linuxAarch64() {
        assertEquals("/native/linux-aarch64/libtstjni.so",
                NativeLoader.resourcePath("Linux", "aarch64"));
    }

    @Test
    void macosAppleSilicon() {
        // arm64 (what a real Apple-Silicon JVM reports) normalizes to aarch64.
        assertEquals("/native/macos-aarch64/libtstjni.dylib",
                NativeLoader.resourcePath("Mac OS X", "arm64"));
        assertEquals("/native/macos-aarch64/libtstjni.dylib",
                NativeLoader.resourcePath("Mac OS X", "aarch64"));
    }

    @Test
    void macosIntel() {
        assertEquals("/native/macos-x86_64/libtstjni.dylib",
                NativeLoader.resourcePath("Mac OS X", "x86_64"));
    }

    @Test
    void windowsX86() {
        // cargo emits tstjni.dll; the JAR resource is libtstjni.dll (renamed
        // at staging) and the loader resolves libtstjni.dll.
        assertEquals("/native/windows-x86_64/libtstjni.dll",
                NativeLoader.resourcePath("Windows 11", "amd64"));
    }
}
