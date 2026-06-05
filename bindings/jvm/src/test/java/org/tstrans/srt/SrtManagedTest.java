package org.tstrans.srt;

import static org.junit.jupiter.api.Assertions.*;
import org.junit.jupiter.api.Test;
import org.tstrans.SrtException;

/**
 * Non-live (socket-free) checks for the managed basic-bytes wrappers. URL /
 * mode validation rejects before any connect, so these run on every platform.
 */
class SrtManagedTest {

    /**
     * A listener-mode URL supplied to {@code ManagedSender.fromUrl} must throw
     * {@code CONFIG_INVALID} — the sender checks {@code mode=caller} up-front.
     */
    @Test
    void managedSenderRejectsListenerModeUrl() {
        var e = assertThrows(
            SrtException.class,
            () -> ManagedSender.fromUrl("srt://127.0.0.1:9000?mode=listener")
        );
        assertEquals(SrtException.Kind.CONFIG_INVALID, e.kind());
    }

    /**
     * A clearly-malformed URL fails {@code SrtUrl::parse} → {@code CONFIG_INVALID}.
     */
    @Test
    void managedSenderRejectsMalformedUrl() {
        var e = assertThrows(
            SrtException.class,
            () -> ManagedSender.fromUrl("not-a-url")
        );
        assertEquals(SrtException.Kind.CONFIG_INVALID, e.kind());
    }

    /**
     * A caller-mode URL supplied to {@code ManagedReceiver.fromUrl} must throw
     * {@code CONFIG_INVALID} — the receiver checks {@code mode=listener} up-front.
     */
    @Test
    void managedReceiverRejectsCallerModeUrl() {
        var e = assertThrows(
            SrtException.class,
            () -> ManagedReceiver.fromUrl("srt://127.0.0.1:9000?mode=caller")
        );
        assertEquals(SrtException.Kind.CONFIG_INVALID, e.kind());
    }

    /**
     * A clearly-malformed URL fails {@code SrtUrl::parse} → {@code CONFIG_INVALID}.
     */
    @Test
    void managedReceiverRejectsMalformedUrl() {
        var e = assertThrows(
            SrtException.class,
            () -> ManagedReceiver.fromUrl("not-a-url")
        );
        assertEquals(SrtException.Kind.CONFIG_INVALID, e.kind());
    }

    /**
     * {@code PolicyArgs.from(null)} flattens the default {@link ReconnectPolicy}:
     * maxAttempts=10 (present), exponential backoff 100ms..10_000ms,
     * gapBufferCapacity=256, overflowPolicy=DROP_OLDEST (ordinal 0).
     */
    @Test
    void policyArgsDefaultsRoundTrip() {
        PolicyArgs p = PolicyArgs.from(null);
        assertTrue(p.maxAttemptsPresent());
        assertEquals(10, p.maxAttempts());
        assertEquals(1, p.backoffKind()); // exponential
        assertEquals(100, p.backoffBaseMs());
        assertEquals(10000, p.backoffMaxMs());
        assertEquals(256, p.gapBufferCapacity());
        assertEquals(0, p.overflowPolicy()); // DROP_OLDEST
    }
}
