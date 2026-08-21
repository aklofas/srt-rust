package org.tstrans.srt;

import java.util.Objects;
import java.util.Optional;

/**
 * Tuning for the managed-reconnect SRT wrappers (sub-wave C). Mirrors
 * {@code tstrans.srt.ReconnectPolicy}. Build with {@link #builder()}.
 *
 * <p>Defaults mirror {@code tst_pipeline::ReconnectPolicy::default()}:
 * maxAttempts=10, backoff=exponential(100ms, 10_000ms), gapBufferCapacity=256,
 * overflowPolicy=DROP_OLDEST, mode=BLOCKING. {@code gapBufferCapacity <= 0} throws
 * {@link IllegalArgumentException} (the tst-py {@code ValueError} analog).
 */
public final class ReconnectPolicy {
    private final Integer maxAttempts; // null = retry forever
    private final BackoffStrategy backoff;
    private final int gapBufferCapacity;
    private final OverflowPolicy overflowPolicy;
    private final ReconnectMode mode;

    private ReconnectPolicy(Builder b) {
        this.maxAttempts = b.maxAttempts;
        this.backoff = b.backoff;
        this.gapBufferCapacity = b.gapBufferCapacity;
        this.overflowPolicy = b.overflowPolicy;
        this.mode = b.mode;
    }

    public static Builder builder() { return new Builder(); }
    /** All-defaults policy. */
    public static ReconnectPolicy defaults() { return builder().build(); }

    /** Maximum reconnect attempts before giving up; empty = retry forever. */
    public Optional<Integer> maxAttempts() { return Optional.ofNullable(maxAttempts); }
    public BackoffStrategy backoff() { return backoff; }
    public int gapBufferCapacity() { return gapBufferCapacity; }
    public OverflowPolicy overflowPolicy() { return overflowPolicy; }
    /** Reconnect-loop placement; default {@link ReconnectMode#BLOCKING}. */
    public ReconnectMode mode() { return mode; }

    public static final class Builder {
        private Integer maxAttempts = 10;
        private BackoffStrategy backoff = BackoffStrategy.defaultStrategy();
        private int gapBufferCapacity = 256;
        private OverflowPolicy overflowPolicy = OverflowPolicy.DROP_OLDEST;
        private ReconnectMode mode = ReconnectMode.BLOCKING;

        /** Pass {@code null} for retry-forever. */
        public Builder maxAttempts(Integer v) { this.maxAttempts = v; return this; }
        public Builder backoff(BackoffStrategy v) {
            this.backoff = Objects.requireNonNull(v, "backoff"); return this;
        }
        public Builder gapBufferCapacity(int v) { this.gapBufferCapacity = v; return this; }
        public Builder overflowPolicy(OverflowPolicy v) {
            this.overflowPolicy = Objects.requireNonNull(v, "overflowPolicy"); return this;
        }
        public Builder mode(ReconnectMode v) {
            this.mode = Objects.requireNonNull(v, "mode"); return this;
        }

        public ReconnectPolicy build() {
            if (gapBufferCapacity <= 0)
                throw new IllegalArgumentException("gapBufferCapacity must be > 0, got " + gapBufferCapacity);
            if (maxAttempts != null && maxAttempts < 0)
                throw new IllegalArgumentException("maxAttempts must be >= 0 or null, got " + maxAttempts);
            return new ReconnectPolicy(this);
        }
    }
}
