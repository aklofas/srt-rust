package org.tstrans.codec;

/**
 * MPEG audio layer (ISO/IEC 11172-3 / 13818-3). Mirrors
 * {@code tst_core::codec::mpegaudio::Layer} and tst-py's
 * {@code tstrans.codec.Layer}.
 */
public enum Layer {
    /** Layer I (384 samples/frame). */
    I,
    /** Layer II (1152 samples/frame). */
    II,
    /** Layer III (MP3; 1152 samples/frame for MPEG-1, 576 for MPEG-2/2.5). */
    III,
}
