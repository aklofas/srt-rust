package org.tstrans.pipeline;

import java.time.Duration;

/** Pairing strategy. Mirrors {@code tstrans.pipeline.PairerMode}. */
public sealed interface PairerMode permits PairerMode.Realtime, PairerMode.Buffered {
    /** Eager pairing; emit on each feed. */
    record Realtime() implements PairerMode {}
    /** Buffer up to {@code maxLag} of arrival skew before forced emit. */
    record Buffered(Duration maxLag) implements PairerMode {
        public Buffered {
            if (maxLag == null) throw new IllegalArgumentException("maxLag must be non-null");
            if (maxLag.isNegative()) throw new IllegalArgumentException("maxLag must be non-negative");
        }
    }
}
