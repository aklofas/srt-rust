package org.tstrans.codec;

/**
 * H.266 / VVC slice type (H.266 V4 §7.4.8 Table 9).
 * Mirrors {@code tst_core::codec::h266::H266SliceType} (and tst-py's
 * {@code tstrans.codec.H266SliceType}).
 *
 * <p>Only three values are defined by the spec. {@link #UNKNOWN} is the
 * open-enum catch-all returned if the Rust parser ever produces a
 * {@code non_exhaustive} variant not yet mapped here, mirroring tst-py's
 * {@code Unknown}. The native parser never returns it today (the light
 * slice-header parser always reports {@link #I} as a sentinel).
 */
public enum H266SliceType {
    /** B slice — bidirectionally predicted. */
    B,
    /** P slice — predicted. */
    P,
    /** I slice — intra-coded. */
    I,
    /** Unrecognised slice type (open-enum catch-all). */
    UNKNOWN,
}
