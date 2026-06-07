package org.tstrans.pipeline;

import java.time.Duration;

/** Pairing tuning. Mirrors {@code tstrans.pipeline.PairerConfig}. Defaults:
 *  Realtime, 300 ms tolerance, 32/32 buffers, link-klv-to-video. */
public record PairerConfig(PairerMode mode, Duration tolerance,
        long maxBufferedKlv, long maxBufferedVideo, boolean linkKlvToVideo) {
    public PairerConfig {
        if (mode == null) throw new IllegalArgumentException("mode must be non-null");
        if (tolerance == null) throw new IllegalArgumentException("tolerance must be non-null");
        if (tolerance.isNegative()) throw new IllegalArgumentException("tolerance must be non-negative");
        if (maxBufferedKlv <= 0) throw new IllegalArgumentException("maxBufferedKlv must be > 0");
        if (mode instanceof PairerMode.Buffered && maxBufferedVideo <= 0)
            throw new IllegalArgumentException("maxBufferedVideo must be > 0 in Buffered mode");
    }
    /** Defaults mirroring the Rust {@code PairerConfig::default()}. */
    public static PairerConfig defaults() {
        return new PairerConfig(new PairerMode.Realtime(), Duration.ofMillis(300), 32, 32, true);
    }
}
