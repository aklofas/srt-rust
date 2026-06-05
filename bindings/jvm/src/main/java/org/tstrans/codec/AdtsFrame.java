package org.tstrans.codec;

import java.nio.ByteBuffer;

/**
 * One decoded AAC ADTS frame (ISO/IEC 13818-7 §1.A). Mirrors
 * {@code tst_core::codec::aac::AdtsFrameOwned} and tst-py's
 * {@code tstrans.codec.AdtsFrame}.
 *
 * <p>{@code rawHeader} carries the 7-byte (no CRC) or 9-byte (with CRC) ADTS
 * header. {@code payload} carries the full frame bytes (header + body) — it
 * sources from the owned {@code body} slice in the Rust type, matching tst-py's
 * {@code payload} getter. Both are heap {@link ByteBuffer}s (JVM-owned copies;
 * no Rust memory escapes).
 *
 * @param profile              AAC profile (Main / LC / SSR / LTP)
 * @param sampleRateHz         sample rate in Hz (e.g. 44100, 48000)
 * @param channelConfiguration raw 3-bit {@code channel_configuration} field;
 *                             {@code 0} = PCE-defined, {@code 1..=7} = canonical
 * @param channelLayout        typed channel layout (see {@link AacChannelLayout})
 * @param frameLengthBytes     total frame byte count (header + body) per the header
 * @param samplesPerFrame      PCM samples in the frame (1024 per raw data block)
 * @param numRawDataBlocks     number of raw data blocks (logical, not wire value)
 * @param hasCrc               {@code true} when a 16-bit CRC follows the fixed header
 * @param mpegVersion          MPEG version (MPEG2 / MPEG4)
 * @param rawHeader            raw ADTS header bytes (7 without CRC, 9 with CRC)
 * @param payload              full frame bytes (header + body)
 */
public record AdtsFrame(
        AacProfile profile,
        long sampleRateHz,
        int channelConfiguration,
        AacChannelLayout channelLayout,
        long frameLengthBytes,
        int samplesPerFrame,
        int numRawDataBlocks,
        boolean hasCrc,
        MpegVersion mpegVersion,
        ByteBuffer rawHeader,
        ByteBuffer payload) implements AudioFrame {}
