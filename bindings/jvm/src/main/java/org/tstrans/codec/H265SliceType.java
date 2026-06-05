package org.tstrans.codec;

/**
 * H.265 / HEVC slice type (H.265 §7.4.7.1 Table 7-7).
 * Mirrors {@code tst_core::codec::h265::H265SliceType} (and tst-py's
 * {@code tstrans.codec.H265SliceType}).
 *
 * <p>Only three values are defined by the spec. {@link #UNKNOWN} is the
 * open-enum catch-all returned if the Rust parser ever produces a
 * {@code non_exhaustive} variant not yet mapped here, mirroring tst-py's
 * {@code Unknown}. The native parser never returns it today.
 */
public enum H265SliceType {
    /** B slice — bidirectionally predicted. */
    B,
    /** P slice — predicted. */
    P,
    /** I slice — intra-coded. */
    I,
    /** Unrecognised slice type (open-enum catch-all). */
    UNKNOWN,
}
