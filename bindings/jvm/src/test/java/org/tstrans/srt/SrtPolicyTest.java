package org.tstrans.srt;

import static org.junit.jupiter.api.Assertions.*;
import org.junit.jupiter.api.Test;

import java.util.Optional;

class SrtPolicyTest {

    // -----------------------------------------------------------------------
    // OverflowPolicy
    // -----------------------------------------------------------------------

    @Test
    void overflowPolicyHasExactlyTwoVariants() {
        assertArrayEquals(
            new OverflowPolicy[]{OverflowPolicy.DROP_OLDEST, OverflowPolicy.REJECT},
            OverflowPolicy.values()
        );
    }

    // -----------------------------------------------------------------------
    // BackoffStrategy
    // -----------------------------------------------------------------------

    @Test
    void constantBackoffHasSymmetricBaseMax() {
        var bs = BackoffStrategy.constant(500);
        assertEquals("constant", bs.kind());
        assertEquals(500L, bs.baseMs());
        assertEquals(500L, bs.maxMs());
    }

    @Test
    void exponentialBackoffPreservesFields() {
        var bs = BackoffStrategy.exponential(100, 10_000);
        assertEquals("exponential", bs.kind());
        assertEquals(100L, bs.baseMs());
        assertEquals(10_000L, bs.maxMs());
    }

    @Test
    void exponentialRejectsInvertedRange() {
        assertThrows(
            IllegalArgumentException.class,
            () -> BackoffStrategy.exponential(10_000, 100)
        );
    }

    @Test
    void constantRejectsNegativeMs() {
        assertThrows(
            IllegalArgumentException.class,
            () -> BackoffStrategy.constant(-1)
        );
    }

    @Test
    void defaultStrategyIsExponential100To10000() {
        var bs = BackoffStrategy.defaultStrategy();
        assertEquals("exponential", bs.kind());
        assertEquals(100L, bs.baseMs());
        assertEquals(10_000L, bs.maxMs());
    }

    @Test
    void backoffStrategyEquality() {
        assertEquals(BackoffStrategy.constant(50), BackoffStrategy.constant(50));
        assertNotEquals(BackoffStrategy.constant(50), BackoffStrategy.constant(51));
        assertEquals(
            BackoffStrategy.exponential(100, 10_000),
            BackoffStrategy.exponential(100, 10_000)
        );
        // kind is part of the equals contract: same base/max but different kind
        // must not compare equal.
        assertNotEquals(
            BackoffStrategy.constant(100),
            BackoffStrategy.exponential(100, 100)
        );
    }

    // -----------------------------------------------------------------------
    // ReconnectPolicy
    // -----------------------------------------------------------------------

    @Test
    void defaultsHaveCorrectFields() {
        var p = ReconnectPolicy.defaults();
        assertEquals(Optional.of(10), p.maxAttempts());
        assertEquals(256, p.gapBufferCapacity());
        assertEquals(OverflowPolicy.DROP_OLDEST, p.overflowPolicy());
        assertEquals("exponential", p.backoff().kind());
        assertEquals(100L, p.backoff().baseMs());
        assertEquals(10_000L, p.backoff().maxMs());
    }

    @Test
    void zeroCapacityThrows() {
        assertThrows(
            IllegalArgumentException.class,
            () -> ReconnectPolicy.builder().gapBufferCapacity(0).build()
        );
    }

    @Test
    void negativeMaxAttemptsThrows() {
        assertThrows(
            IllegalArgumentException.class,
            () -> ReconnectPolicy.builder().maxAttempts(-1).build()
        );
    }

    @Test
    void nullMaxAttemptsProducesEmptyOptional() {
        var p = ReconnectPolicy.builder().maxAttempts(null).build();
        assertTrue(p.maxAttempts().isEmpty());
    }

    @Test
    void customFieldsRoundTrip() {
        var p = ReconnectPolicy.builder()
            .maxAttempts(5)
            .backoff(BackoffStrategy.constant(200))
            .gapBufferCapacity(64)
            .overflowPolicy(OverflowPolicy.REJECT)
            .build();
        assertEquals(Optional.of(5), p.maxAttempts());
        assertEquals(BackoffStrategy.constant(200), p.backoff());
        assertEquals(64, p.gapBufferCapacity());
        assertEquals(OverflowPolicy.REJECT, p.overflowPolicy());
    }
}
