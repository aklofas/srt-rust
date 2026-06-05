package org.tstrans.codec;

/**
 * H.264 entropy coding mode signalled in the PPS.
 * Mirrors {@code tst_core::codec::h264::EntropyCodingMode} (and tst-py's
 * {@code tstrans.codec.EntropyCodingMode}).
 */
public enum EntropyCodingMode {
    /** Context-Adaptive Variable Length Coding (Baseline/Main profiles). */
    CAVLC,
    /** Context-Adaptive Binary Arithmetic Coding (Main/High profiles). */
    CABAC,
}
