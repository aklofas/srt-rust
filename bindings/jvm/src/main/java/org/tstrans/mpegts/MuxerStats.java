package org.tstrans.mpegts;

/**
 * Frozen muxer statistics snapshot. Mirror of the 4 surfaced getters on
 * {@code tstrans.mpegts.MuxerStats}. All counters are widened to {@code long}
 * (Java has no unsigned types — {@code u32} zero-extends, {@code u64}
 * reinterprets the bit pattern).
 *
 * <p>The per-stream push-counter map carried by the underlying Rust
 * {@code MuxerStats} is <strong>not</strong> surfaced here, matching tst-py.
 *
 * @param tsPacketsEmitted            cumulative TS packets emitted
 * @param tsBytesEmitted             cumulative TS bytes emitted
 * @param programsConfigured        number of programs (PAT entries) configured
 * @param subtitleStreamsConfigured number of subtitle streams configured
 */
public record MuxerStats(
        long tsPacketsEmitted,
        long tsBytesEmitted,
        long programsConfigured,
        long subtitleStreamsConfigured) {}
