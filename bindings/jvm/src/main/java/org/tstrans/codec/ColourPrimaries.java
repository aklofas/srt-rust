package org.tstrans.codec;

/**
 * ITU-T H.273 V4 §8.1 Table 2 — colour primaries.
 * Mirrors {@code tstrans.codec.ColourPrimaries}.
 *
 * <p>{@link #RESERVED} collapses the Rust {@code Reserved(u8)} variant (raw
 * value not preserved).
 */
public enum ColourPrimaries {
    BT709, UNSPECIFIED, BT470M, BT470BG, SMPTE170M, SMPTE240M, FILM, BT2020,
    SMPTE_ST428, SMPTE_RP431_2, SMPTE_EG432_1, EBU3213E, RESERVED
}
