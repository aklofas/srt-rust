package org.tstrans.pipeline;

/** Frozen pairing-counter snapshot. Mirrors {@code tstrans.pipeline.Pairer.stats()}.
 *  Counters widened to {@code long} (Rust {@code u64}). */
public record PairerStats(long paired, long unpairedVideo, long unpairedKlv, long passThrough) {}
