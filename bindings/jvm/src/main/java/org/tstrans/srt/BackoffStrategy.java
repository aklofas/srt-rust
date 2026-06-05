package org.tstrans.srt;

import java.util.Objects;

/**
 * Backoff strategy for reconnect attempts. Mirrors {@code tstrans.srt.BackoffStrategy}.
 * Construct via {@link #constant(long)} or {@link #exponential(long, long)}.
 * The accessors ({@link #kind()}, {@link #baseMs()}, {@link #maxMs()}) work
 * uniformly across both variants — for {@code constant}, {@code baseMs == maxMs}.
 */
public final class BackoffStrategy {
    private final String kind; // "constant" | "exponential"
    private final long baseMs;
    private final long maxMs;

    private BackoffStrategy(String kind, long baseMs, long maxMs) {
        this.kind = kind;
        this.baseMs = baseMs;
        this.maxMs = maxMs;
    }

    /** Fixed wait between reconnect attempts. */
    public static BackoffStrategy constant(long ms) {
        if (ms < 0) throw new IllegalArgumentException("ms must be >= 0, got " + ms);
        return new BackoffStrategy("constant", ms, ms);
    }

    /** Exponential backoff: wait = base * 2^(attempt-1), capped at max. */
    public static BackoffStrategy exponential(long baseMs, long maxMs) {
        if (baseMs < 0) throw new IllegalArgumentException("baseMs must be >= 0, got " + baseMs);
        if (maxMs < 0) throw new IllegalArgumentException("maxMs must be >= 0, got " + maxMs);
        if (maxMs < baseMs)
            throw new IllegalArgumentException("maxMs must be >= baseMs (" + maxMs + " < " + baseMs + ")");
        return new BackoffStrategy("exponential", baseMs, maxMs);
    }

    /** Default mirrors {@code tst_pipeline::BackoffStrategy::default()}: exponential 100ms..10_000ms. */
    public static BackoffStrategy defaultStrategy() {
        return exponential(100, 10_000);
    }

    public String kind() { return kind; }
    public long baseMs() { return baseMs; }
    public long maxMs() { return maxMs; }

    @Override public String toString() {
        return kind.equals("constant")
            ? "BackoffStrategy.constant(ms=" + baseMs + ")"
            : "BackoffStrategy.exponential(baseMs=" + baseMs + ", maxMs=" + maxMs + ")";
    }

    @Override public boolean equals(Object o) {
        if (!(o instanceof BackoffStrategy b)) return false;
        return baseMs == b.baseMs && maxMs == b.maxMs && kind.equals(b.kind);
    }
    @Override public int hashCode() { return Objects.hash(kind, baseMs, maxMs); }
}
