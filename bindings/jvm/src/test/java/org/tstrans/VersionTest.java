package org.tstrans;

import static org.junit.jupiter.api.Assertions.assertNotNull;
import static org.junit.jupiter.api.Assertions.assertTrue;

import org.junit.jupiter.api.Test;

class VersionTest {

    @Test
    void versionStringCrossesJniBoundary() {
        String version = Version.versionString();
        assertNotNull(version, "versionString() returned null across JNI");
        assertTrue(
                version.matches("\\d+\\.\\d+\\.\\d+.*"),
                "expected a semver-shaped version from Rust, got: " + version);
    }
}
