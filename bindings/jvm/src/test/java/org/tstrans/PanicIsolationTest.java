package org.tstrans;

import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.condition.EnabledIfSystemProperty;
import org.tstrans.internal.PanicProbe;

import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

/**
 * Proves a Rust panic inside a JNI native is isolated as a RuntimeException
 * rather than aborting the JVM. Runs only when the cdylib was built with the
 * {@code jni-test-hooks} cargo feature (Gradle {@code -PjniTestHooks=true}),
 * so the shipped JAR never needs the probe symbol.
 */
@EnabledIfSystemProperty(named = "tst.jniTestHooks", matches = "true")
class PanicIsolationTest {

    @Test
    void rustPanicBecomesRuntimeException() {
        RuntimeException ex =
                assertThrows(RuntimeException.class, PanicProbe::nForcePanic);
        assertTrue(
                ex.getMessage() != null
                        && ex.getMessage().contains("intentional panic"),
                "expected isolated panic message, got: " + ex.getMessage());
    }
}
