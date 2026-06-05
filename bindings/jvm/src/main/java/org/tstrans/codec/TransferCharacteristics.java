package org.tstrans.codec;

/**
 * ITU-T H.273 V4 §8.2 Table 3 — transfer characteristics.
 * Mirrors {@code tstrans.codec.TransferCharacteristics}.
 *
 * <p>{@link #RESERVED} collapses the Rust {@code Reserved(u8)} variant.
 */
public enum TransferCharacteristics {
    BT709, UNSPECIFIED, GAMMA22, GAMMA28, SMPTE170M, SMPTE240M, LINEAR, LOG100,
    LOG_SQRT, IEC61966_2_4, BT1361E, IEC61966_2_1, BT2020_BITS10, BT2020_BITS12,
    SMPTE_ST2084, SMPTE_ST428, ARIB_STD_B67, RESERVED
}
