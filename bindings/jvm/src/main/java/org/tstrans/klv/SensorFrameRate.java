package org.tstrans.klv;

/**
 * MISB ST 0601.19 §8.127 Item 127 — Sensor Frame Rate Pack: frame rate as a
 * numerator/denominator ratio. On the wire, a value absent {@code denominator}
 * defaults to 1 (whole-number fps) — decode/encode in {@link Klv} apply that
 * default; this record always carries both fields explicitly.
 *
 * @param numerator   BER-OID numerator
 * @param denominator BER-OID denominator
 */
public record SensorFrameRate(long numerator, long denominator) {

    /** Frames per second as {@code numerator / denominator}. */
    public double fps() {
        return (double) numerator / (double) denominator;
    }
}
