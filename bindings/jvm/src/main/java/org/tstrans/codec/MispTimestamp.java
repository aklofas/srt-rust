package org.tstrans.codec;

import org.tstrans.CodecParseException;

/**
 * One MISB ST 0604 MISP timestamp, carried in a video SEI NAL.
 * Mirrors {@code tst_core::codec::misp_time::MispTimestamp}.
 *
 * <p>The {@link #value()} field crosses JNI as a {@code jlong} (Java
 * {@code long}), and is treated as an <em>unsigned 64-bit</em> integer —
 * i.e. bit-pattern reinterpretation, no sign extension. Compare with
 * {@link Long#compareUnsigned(long, long)} and format with
 * {@link Long#toUnsignedString(long)} when the full unsigned range is needed.
 *
 * <p>The {@link #timeStatus()} field is {@code int} because Java has no
 * unsigned byte type; only the low 8 bits are meaningful (values 0–255),
 * matching the ST 0603 Time Status byte definition.
 *
 * @param kind       whether {@link #value()} is microseconds ({@link MispTimeKind#MICRO})
 *                   or nanoseconds ({@link MispTimeKind#NANO})
 * @param timeStatus MISB ST 0603 Time Status byte (0–255); only the low 8
 *                   bits are used
 * @param value      timestamp magnitude; treated as unsigned 64-bit
 */
public record MispTimestamp(MispTimeKind kind, int timeStatus, long value) {

    /**
     * Construct a microsecond-precision MISP timestamp.
     * Valid for H.264 and H.265 per ST 0604.6 §7/§12.1.
     *
     * @param valueUs    microseconds since the MISP epoch (unsigned 64-bit
     *                   crossing; high-bit values are valid)
     * @param timeStatus ST 0603 Time Status byte (0–255)
     * @return a {@code MispTimestamp} with {@link MispTimeKind#MICRO}
     */
    public static MispTimestamp micros(long valueUs, int timeStatus) {
        return new MispTimestamp(MispTimeKind.MICRO, timeStatus, valueUs);
    }

    /**
     * Construct a nanosecond-precision MISP timestamp.
     * H.265-only per ST 0604.6 §12.2; passing this to
     * {@link org.tstrans.mpegts.Muxer#pushVideoMispTo} on an H.264 stream
     * will throw {@link org.tstrans.MuxException} with kind
     * {@code INVALID_USAGE}.
     *
     * @param valueNs    nanoseconds since the MISP epoch (unsigned 64-bit
     *                   crossing)
     * @param timeStatus ST 0603 Time Status byte (0–255)
     * @return a {@code MispTimestamp} with {@link MispTimeKind#NANO}
     */
    public static MispTimestamp nanos(long valueNs, int timeStatus) {
        return new MispTimestamp(MispTimeKind.NANO, timeStatus, valueNs);
    }

    /**
     * Scan an Annex-B access unit for the first MISB ST 0604 MISP timestamp
     * SEI and return it, or {@code null} when no MISP SEI is present.
     *
     * <p>Liberal on input: all three ST 0604.6 identifiers (H.264 microsecond,
     * H.265 microsecond, H.265 nanosecond) are matched regardless of the
     * supplied {@code codec}; prefix and suffix SEI positions are both scanned;
     * non-MISP SEI content is skipped.
     *
     * @param au    Annex-B access unit bytes (H.264 or H.265 encoded)
     * @param codec the video codec of {@code au} (from
     *              {@link org.tstrans.mpegts.DemuxEvent.Video#codec()})
     * @return the first MISP timestamp found, or {@code null} if absent
     * @throws CodecParseException if a MISP SEI identifier was matched but the
     *         payload is malformed (truncated payload or bad ST 0604.6 §7.4
     *         guard byte); absence is signalled by returning {@code null}, not
     *         by throwing
     */
    public static MispTimestamp extract(
            byte[] au,
            org.tstrans.mpegts.VideoCodec codec)
            throws CodecParseException {
        return Codec.extractMispTimestamp(au, codec.ordinal());
    }
}
