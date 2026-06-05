package org.tstrans.codec;

/**
 * MPEG audio version (ISO/IEC 11172-3 / 13818-3). Mirrors
 * {@code tst_core::codec::mpegaudio::Version} and tst-py's
 * {@code tstrans.codec.Version}.
 *
 * <p>MPEG-2.5 is the de-facto half-rate extension (8 / 11.025 / 12 kHz
 * Layer III); not part of any ratified ISO spec but ubiquitous in consumer
 * MP3 streams.
 */
public enum Version {
    /** MPEG-1 (ISO/IEC 11172-3). */
    MPEG1,
    /** MPEG-2 (ISO/IEC 13818-3, low sampling rates). */
    MPEG2,
    /** MPEG-2.5 (de-facto half-rate extension). */
    MPEG2_5,
}
