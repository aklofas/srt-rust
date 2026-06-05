package org.tstrans.codec;

/**
 * ADTS MPEG version bit (one bit; {@code 0} = MPEG-4, {@code 1} = MPEG-2).
 * Mirrors {@code tst_core::codec::aac::MpegVersion} (and tst-py's
 * {@code tstrans.codec.MpegVersion}).
 */
public enum MpegVersion {
    /** MPEG-2 AAC (ADTS {@code ID} bit = 1). */
    MPEG2,
    /** MPEG-4 AAC (ADTS {@code ID} bit = 0). */
    MPEG4,
}
