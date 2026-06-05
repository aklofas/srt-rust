package org.tstrans.codec;

/**
 * MPEG audio channel mode (header bits 25-26; ISO/IEC 11172-3 / 13818-3).
 * Mirrors {@code tst_core::codec::mpegaudio::ChannelMode} and tst-py's
 * {@code tstrans.codec.ChannelMode}.
 */
public enum ChannelMode {
    /** Stereo (2 channels). */
    STEREO,
    /** Joint stereo (2 channels). */
    JOINT_STEREO,
    /** Dual channel (2 independent channels). */
    DUAL_CHANNEL,
    /** Single channel / mono (1 channel). */
    MONO,
}
