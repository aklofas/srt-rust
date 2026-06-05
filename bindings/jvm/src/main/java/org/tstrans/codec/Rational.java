package org.tstrans.codec;

/**
 * Numerator/denominator pair (no implicit reduction).
 * Mirrors {@code tstrans.codec.Rational}.
 *
 * <p>Both fields are Java {@code long} because the underlying Rust fields are
 * {@code u32} (which do not fit a Java {@code int} unsigned).
 *
 * @param num numerator
 * @param den denominator
 */
public record Rational(long num, long den) {
    /** @return {@code num / den} as a floating-point value. */
    public double asFloat() {
        return (double) num / (double) den;
    }
}
