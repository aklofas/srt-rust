package org.tstrans.codec;

import java.nio.ByteBuffer;

/**
 * One decoded MPEG audio frame (ISO/IEC 11172-3 / 13818-3 — MPEG-1/2/2.5
 * Layer I/II/III). Mirrors {@code tst_core::codec::mpegaudio::FrameOwned} and
 * tst-py's {@code tstrans.codec.Mpeg2AudioFrame}.
 *
 * <p>{@code rawHeader} carries the fixed 4-byte MPEG audio frame header.
 * {@code payload} carries the full frame bytes (header + body, including the
 * CRC bytes when {@link #hasCrc()}) — it sources from the owned {@code body}
 * slice in the Rust type, matching tst-py's {@code payload} getter. Both are
 * heap {@link ByteBuffer}s (JVM-owned copies; no Rust memory escapes).
 *
 * @param layer            MPEG audio layer (I / II / III)
 * @param version          MPEG version (MPEG1 / MPEG2 / MPEG2_5)
 * @param bitrateKbps      bitrate in kilobits per second (e.g. 128, 192, 320)
 * @param sampleRateHz     sample rate in Hz (e.g. 44100, 48000)
 * @param channelMode      channel mode (Stereo / JointStereo / DualChannel / Mono)
 * @param channels         number of audio channels (1 for Mono, 2 otherwise)
 * @param frameLengthBytes total frame byte count as computed from the header
 * @param samplesPerFrame  PCM samples in the frame (384 / 576 / 1152 per version+layer)
 * @param hasCrc           {@code true} when a 16-bit CRC follows the 4-byte header
 * @param rawHeader        raw 4-byte MPEG audio frame header
 * @param payload          full frame bytes (header + body)
 */
public record Mpeg2AudioFrame(
        Layer layer,
        Version version,
        long bitrateKbps,
        long sampleRateHz,
        ChannelMode channelMode,
        int channels,
        long frameLengthBytes,
        int samplesPerFrame,
        boolean hasCrc,
        ByteBuffer rawHeader,
        ByteBuffer payload) implements AudioFrame {}
