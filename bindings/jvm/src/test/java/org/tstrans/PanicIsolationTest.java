package org.tstrans;

import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.condition.EnabledIfSystemProperty;
import org.tstrans.internal.PanicProbe;

import static org.junit.jupiter.api.Assertions.assertDoesNotThrow;
import static org.junit.jupiter.api.Assertions.assertNotEquals;
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

    @Test
    @EnabledIfSystemProperty(named = "tst.jniTestHooks", matches = "true")
    void mutatingPanicPoisonsHandleEndToEnd() {
        long handle = PanicProbe.nOpenHandle();
        assertNotEquals(0L, handle);

        // (1) A non-panicking mutation succeeds.
        assertDoesNotThrow(() -> PanicProbe.nMutateMaybePanic(handle, false));

        // (2) A mutation that panics mid-way is caught and surfaces as a thrown
        //     exception (RuntimeException from the outer jni_catch).
        assertThrows(RuntimeException.class,
                () -> PanicProbe.nMutateMaybePanic(handle, true));

        // (3) The handle is now poisoned: any later op throws
        //     IllegalStateException (deterministic closed/poisoned), never a
        //     reuse of the torn state.
        assertThrows(IllegalStateException.class,
                () -> PanicProbe.nMutateMaybePanic(handle, false));

        // (4) Closing a poisoned handle is a safe no-op.
        assertDoesNotThrow(() -> PanicProbe.nCloseHandle(handle));
    }
}
