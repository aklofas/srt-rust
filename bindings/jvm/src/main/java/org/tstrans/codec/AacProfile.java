package org.tstrans.codec;

/**
 * AAC profile per ADTS ISO/IEC 13818-7 §1.A Table 8. Mirrors
 * {@code tst_core::codec::aac::AacProfile} (and tst-py's
 * {@code tstrans.codec.AacProfile}).
 *
 * <p>The {@code profile} field's interpretation depends on the ADTS {@code ID}
 * bit (MPEG version): for MPEG-4 it is an audio object type minus one; for
 * MPEG-2 it is the legacy AAC profile. Most real-world ADTS encodes
 * {@link #LC} regardless of the encoder's MPEG-4 object type.
 */
public enum AacProfile {
    /** Main profile. */
    MAIN,
    /** Low Complexity (AAC-LC) — the common case. */
    LC,
    /** Scalable Sampling Rate. */
    SSR,
    /** Long-Term Prediction (MPEG-4 only). */
    LTP,
}
