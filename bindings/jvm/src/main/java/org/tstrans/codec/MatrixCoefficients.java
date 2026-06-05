package org.tstrans.codec;

/**
 * ITU-T H.273 V4 §8.3 Table 4 — matrix coefficients.
 * Mirrors {@code tstrans.codec.MatrixCoefficients}.
 *
 * <p>{@link #RESERVED} collapses the Rust {@code Reserved(u8)} variant.
 */
public enum MatrixCoefficients {
    IDENTITY, BT709, UNSPECIFIED, FCC_MC, BT470BG, SMPTE170M, SMPTE240M, YCGCO,
    BT2020_NON_CONSTANT, BT2020_CONSTANT, SMPTE_ST2085, CHROMA_DERIVED_NON_CONSTANT,
    CHROMA_DERIVED_CONSTANT, ICTCP, IPT_C2, YCGCO_RE, YCGCO_RO, RESERVED
}
