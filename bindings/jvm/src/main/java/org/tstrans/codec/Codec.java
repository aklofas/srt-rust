package org.tstrans.codec;

import java.util.List;
import org.tstrans.CodecParseException;

/**
 * Static facade for typed codec parameter-set / payload-unit parsing.
 * Mirrors tst-py's {@code tstrans.codec} free functions.
 *
 * <p>Parser entry points for further codecs (H.265 / H.266 / AV1 / audio) are
 * added in follow-on tasks of the codec wave; this facade currently exposes the
 * H.264 parameter-set / slice-header parsers.
 */
public final class Codec {
    private Codec() {}

    static {
        org.tstrans.NativeLoader.load();
    }

    // --- H.264 native entry points ---------------------------------------
    // The native methods cannot declare `throws`; on a parse error they leave a
    // pending CodecParseException and return null. The public wrappers below
    // declare `throws CodecParseException` so the pending exception propagates.

    private static native H264Sps nParseH264Sps(byte[] rbsp);

    private static native H264Pps nParseH264Pps(byte[] rbsp);

    private static native H264SliceHeaderLight nParseH264SliceHeaderLight(
            byte[] rbsp, H264Sps sps, int nalUnitType);

    private static native H264ParameterSets nParseH264ParameterSets(List<NalUnit> nals);

    /**
     * Parse a single H.264 SPS RBSP.
     *
     * <p>{@code rbsp} must be the raw RBSP body — Annex-B start code stripped,
     * NAL header byte stripped, emulation-prevention bytes preserved (matches
     * {@link NalUnit#h264}'s {@code payload}).
     *
     * @param rbsp the SPS RBSP body
     * @return the parsed SPS
     * @throws CodecParseException on a malformed or truncated bitstream
     */
    public static H264Sps parseH264Sps(byte[] rbsp) throws CodecParseException {
        return nParseH264Sps(rbsp);
    }

    /**
     * Parse a single H.264 PPS RBSP. Same input contract as
     * {@link #parseH264Sps(byte[])}.
     *
     * @param rbsp the PPS RBSP body
     * @return the parsed PPS
     * @throws CodecParseException on a malformed or truncated bitstream
     */
    public static H264Pps parseH264Pps(byte[] rbsp) throws CodecParseException {
        return nParseH264Pps(rbsp);
    }

    /**
     * Parse a light H.264 slice header from a slice NAL's RBSP body.
     *
     * <p>{@code sps} is optional SPS context — when non-null, {@code frameNum}
     * is read from the bitstream using the bit width
     * {@code log2MaxFrameNumMinus4 + 4}; when null, {@code frameNum} is null.
     * {@code nalUnitType} is the 5-bit NAL type ({@code & 0x1F}) used to derive
     * {@code idr} ({@code == 5}).
     *
     * @param rbsp        the slice NAL RBSP body
     * @param sps         SPS context, or {@code null}
     * @param nalUnitType the 5-bit NAL unit type
     * @return the parsed light slice header
     * @throws CodecParseException on a malformed or truncated bitstream
     */
    public static H264SliceHeaderLight parseH264SliceHeaderLight(
            byte[] rbsp, H264Sps sps, int nalUnitType) throws CodecParseException {
        return nParseH264SliceHeaderLight(rbsp, sps, nalUnitType);
    }

    /**
     * Parse all H.264 SPS and PPS NAL units from a list of {@link NalUnit}.
     *
     * <p>Non-H.264 entries (and non-SPS/PPS H.264 NALs) are silently skipped.
     * Parsing is partial-success-tolerant; a {@link CodecParseException} is
     * raised only when every parameter-set NAL fails to parse.
     *
     * @param nals the NAL units to scan
     * @return the collected SPS/PPS maps
     * @throws CodecParseException when no parameter set could be parsed
     */
    public static H264ParameterSets parseH264ParameterSets(List<NalUnit> nals)
            throws CodecParseException {
        return nParseH264ParameterSets(nals);
    }
}
