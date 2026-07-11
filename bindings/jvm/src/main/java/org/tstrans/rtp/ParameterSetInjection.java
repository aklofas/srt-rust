package org.tstrans.rtp;

/**
 * Controls whether out-of-band SPS/PPS are injected before IDR frames.
 *
 * <p>Mirrors {@code tst_rtp::ParameterSetInjection}. Ordinal values are
 * stable across releases — the JNI layer passes these by ordinal to Rust.
 *
 * <ul>
 *   <li>{@link #NONE} — pass NALUs through exactly as received.
 *   <li>{@link #BEFORE_IDR} — inject cached SPS and PPS NALUs before every IDR
 *       frame (the default). Enables random-access decoding without re-signalling
 *       parameter sets out-of-band.
 * </ul>
 */
public enum ParameterSetInjection {
    /** No injection — NALUs are passed through exactly as received. Ordinal 0. */
    NONE,
    /** Inject cached SPS + PPS before every IDR frame. Ordinal 1. Default. */
    BEFORE_IDR
}
