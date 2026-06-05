package org.tstrans.codec;

/**
 * H.264 slice type, normalised via {@code slice_type % 5} (H.264 §7.4.3).
 * Mirrors {@code tst_core::codec::h264::H264SliceType} (and tst-py's
 * {@code tstrans.codec.H264SliceType}).
 *
 * <p>{@link #UNKNOWN} is the open-enum catch-all: it is returned when the Rust
 * parser produces a {@code #non_exhaustive} variant not yet mapped to a
 * constant here, mirroring tst-py's {@code Unknown}. The native parser never
 * returns it today.
 */
public enum H264SliceType {
    /** P slice — predicted. */
    P,
    /** B slice — bidirectionally predicted. */
    B,
    /** I slice — intra-coded. */
    I,
    /** SP slice — switching P. */
    SP,
    /** SI slice — switching I. */
    SI,
    /** Unrecognised slice type (open-enum catch-all). */
    UNKNOWN,
}
