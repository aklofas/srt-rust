package org.tstrans.codec;

import java.util.List;
import org.tstrans.CodecParseException;

/**
 * Static facade for typed codec parameter-set / payload-unit parsing.
 * Mirrors tst-py's {@code tstrans.codec} free functions.
 *
 * <p>Parser entry points for further codecs (H.266 / AV1 / audio) are added in
 * follow-on tasks of the codec wave; this facade currently exposes the H.264 and
 * H.265 parameter-set / slice-header parsers.
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

    // --- H.265 native entry points ---------------------------------------

    private static native H265Sps nParseH265Sps(byte[] rbsp);

    private static native H265Pps nParseH265Pps(byte[] rbsp);

    private static native H265Vps nParseH265Vps(byte[] rbsp);

    private static native H265SliceHeaderLight nParseH265SliceHeaderLight(
            byte[] rbsp, H265Sps sps, int nalUnitType);

    private static native H265ParameterSets nParseH265ParameterSets(List<NalUnit> nals);

    /**
     * Parse a single H.265 SPS RBSP.
     *
     * <p>{@code rbsp} must be the raw RBSP body — Annex-B start code stripped,
     * NAL header (2 bytes for H.265) stripped, emulation-prevention bytes
     * preserved (matches {@link NalUnit#h265}'s {@code payload}).
     *
     * @param rbsp the SPS RBSP body
     * @return the parsed SPS
     * @throws CodecParseException on a malformed or truncated bitstream
     */
    public static H265Sps parseH265Sps(byte[] rbsp) throws CodecParseException {
        return nParseH265Sps(rbsp);
    }

    /**
     * Parse a single H.265 PPS RBSP. Same input contract as
     * {@link #parseH265Sps(byte[])}.
     *
     * @param rbsp the PPS RBSP body
     * @return the parsed PPS
     * @throws CodecParseException on a malformed or truncated bitstream
     */
    public static H265Pps parseH265Pps(byte[] rbsp) throws CodecParseException {
        return nParseH265Pps(rbsp);
    }

    /**
     * Parse a single H.265 VPS RBSP. Same input contract as
     * {@link #parseH265Sps(byte[])}.
     *
     * @param rbsp the VPS RBSP body
     * @return the parsed VPS
     * @throws CodecParseException on a malformed or truncated bitstream
     */
    public static H265Vps parseH265Vps(byte[] rbsp) throws CodecParseException {
        return nParseH265Vps(rbsp);
    }

    /**
     * Parse a light H.265 slice segment header from a slice NAL's RBSP body.
     *
     * <p>{@code sps} is optional SPS context — when non-null,
     * {@code picOrderCntLsb} is read from the bitstream using the bit width
     * {@code log2MaxPicOrderCntLsbMinus4 + 4}; when null, {@code picOrderCntLsb}
     * is null. {@code nalUnitType} is the 6-bit NAL type used to derive
     * {@code idr} (IDR_W_RADL=19 or IDR_N_LP=20) and to gate IRAP-specific
     * fields.
     *
     * @param rbsp        the slice NAL RBSP body
     * @param sps         SPS context, or {@code null}
     * @param nalUnitType the 6-bit NAL unit type
     * @return the parsed light slice header
     * @throws CodecParseException on a malformed or truncated bitstream
     */
    public static H265SliceHeaderLight parseH265SliceHeaderLight(
            byte[] rbsp, H265Sps sps, int nalUnitType) throws CodecParseException {
        return nParseH265SliceHeaderLight(rbsp, sps, nalUnitType);
    }

    /**
     * Parse all H.265 VPS, SPS, and PPS NAL units from a list of
     * {@link NalUnit}.
     *
     * <p>Non-H.265 entries (and non-parameter-set H.265 NALs) are silently
     * skipped. Parsing is partial-success-tolerant; a {@link CodecParseException}
     * is raised only when every parameter-set NAL fails to parse.
     *
     * @param nals the NAL units to scan
     * @return the collected VPS/SPS/PPS maps
     * @throws CodecParseException when no parameter set could be parsed
     */
    public static H265ParameterSets parseH265ParameterSets(List<NalUnit> nals)
            throws CodecParseException {
        return nParseH265ParameterSets(nals);
    }

    // --- H.266 native entry points ---------------------------------------

    private static native H266Sps nParseH266Sps(byte[] rbsp);

    private static native H266Pps nParseH266Pps(byte[] rbsp);

    private static native H266Vps nParseH266Vps(byte[] rbsp);

    private static native H266SliceHeaderLight nParseH266SliceHeaderLight(
            byte[] rbsp, H266Sps sps, int nalUnitType);

    private static native H266ParameterSets nParseH266ParameterSets(List<NalUnit> nals);

    /**
     * Parse a single H.266 / VVC SPS RBSP.
     *
     * <p>{@code rbsp} must be the raw RBSP body — Annex-B start code stripped,
     * NAL header (2 bytes for H.266) stripped, emulation-prevention bytes
     * preserved (matches {@link NalUnit#h266}'s {@code payload}).
     *
     * @param rbsp the SPS RBSP body
     * @return the parsed SPS
     * @throws CodecParseException on a malformed or truncated bitstream
     */
    public static H266Sps parseH266Sps(byte[] rbsp) throws CodecParseException {
        return nParseH266Sps(rbsp);
    }

    /**
     * Parse a single H.266 / VVC PPS RBSP. Same input contract as
     * {@link #parseH266Sps(byte[])}.
     *
     * @param rbsp the PPS RBSP body
     * @return the parsed PPS
     * @throws CodecParseException on a malformed or truncated bitstream
     */
    public static H266Pps parseH266Pps(byte[] rbsp) throws CodecParseException {
        return nParseH266Pps(rbsp);
    }

    /**
     * Parse a single H.266 / VVC VPS RBSP. Same input contract as
     * {@link #parseH266Sps(byte[])}.
     *
     * @param rbsp the VPS RBSP body
     * @return the parsed VPS
     * @throws CodecParseException on a malformed or truncated bitstream
     */
    public static H266Vps parseH266Vps(byte[] rbsp) throws CodecParseException {
        return nParseH266Vps(rbsp);
    }

    /**
     * Parse a light H.266 / VVC slice header from a slice NAL's RBSP body.
     *
     * <p>{@code sps} is optional SPS context — when non-null, it is used to
     * recover the {@code picOrderCntLsb} bit width for the (deferred) non-IDR
     * path; when null, {@code picOrderCntLsb} is null for non-IDR slices.
     * {@code nalUnitType} is the NAL type used to derive {@code idr}
     * (IDR_W_RADL=7 or IDR_N_LP=8). Note: {@code sliceType} and {@code ppsId}
     * are returned as sentinels ({@code I} / {@code 0}) — see
     * {@link H266SliceHeaderLight}.
     *
     * @param rbsp        the slice NAL RBSP body
     * @param sps         SPS context, or {@code null}
     * @param nalUnitType the NAL unit type
     * @return the parsed light slice header
     * @throws CodecParseException on a malformed or truncated bitstream
     */
    public static H266SliceHeaderLight parseH266SliceHeaderLight(
            byte[] rbsp, H266Sps sps, int nalUnitType) throws CodecParseException {
        return nParseH266SliceHeaderLight(rbsp, sps, nalUnitType);
    }

    /**
     * Parse all H.266 / VVC VPS, SPS, and PPS NAL units from a list of
     * {@link NalUnit}.
     *
     * <p>Non-H.266 entries (and non-parameter-set H.266 NALs) are silently
     * skipped. Parsing is partial-success-tolerant; a {@link CodecParseException}
     * is raised only when every parameter-set NAL fails to parse. Unlike the
     * H.265 variant, the result is backed by ordered lists, not maps.
     *
     * @param nals the NAL units to scan
     * @return the collected VPS/SPS/PPS lists
     * @throws CodecParseException when no parameter set could be parsed
     */
    public static H266ParameterSets parseH266ParameterSets(List<NalUnit> nals)
            throws CodecParseException {
        return nParseH266ParameterSets(nals);
    }

    // --- AV1 native entry points -----------------------------------------

    private static native Av1SequenceHeader nParseAv1SequenceHeader(byte[] payload);

    private static native Av1FrameHeaderLight nParseAv1FrameHeaderLight(
            byte[] payload, Av1SequenceHeader seq);

    // No `throws`: parse_obu_stream is infallible — failures are collected into
    // the returned Av1ObuStream.unparseable rather than thrown.
    private static native Av1ObuStream nParseAv1ObuStream(List<Obu> obus);

    /**
     * Parse a single AV1 Sequence Header OBU body.
     *
     * <p>{@code payload} carries the OBU body bytes — the OBU header byte and
     * any LEB128 {@code obu_size} prefix are stripped (as {@link Obu#payload()}
     * provides from a demuxed stream).
     *
     * @param payload the Sequence Header OBU body
     * @return the parsed Sequence Header
     * @throws CodecParseException on a malformed or truncated bitstream
     */
    public static Av1SequenceHeader parseAv1SequenceHeader(byte[] payload)
            throws CodecParseException {
        return nParseAv1SequenceHeader(payload);
    }

    /**
     * Parse a light AV1 Frame Header OBU body.
     *
     * <p>{@code payload} carries the OBU body bytes. {@code seq} is the
     * <em>required</em> Sequence Header context — it must correspond to the
     * Sequence Header that precedes this Frame Header in the bitstream. Use
     * {@link #parseAv1SequenceHeader(byte[])} to obtain it.
     *
     * <p>Light scope: extracts {@code frameType} + {@code showFrame} +
     * {@code showExistingFrame} only; {@code frameSize} is always {@code null}.
     *
     * @param payload the Frame Header OBU body
     * @param seq     the preceding Sequence Header context (required)
     * @return the parsed light Frame Header
     * @throws CodecParseException on a malformed or truncated bitstream
     */
    public static Av1FrameHeaderLight parseAv1FrameHeaderLight(byte[] payload, Av1SequenceHeader seq)
            throws CodecParseException {
        return nParseAv1FrameHeaderLight(payload, seq);
    }

    /**
     * Walk a list of {@link Obu} objects and collect typed AV1 structs.
     *
     * <p>Partial-success-tolerant and <em>infallible</em>: OBUs that fail to
     * parse accumulate in {@link Av1ObuStream#unparseable()} rather than
     * aborting the walk — this method never throws. TemporalDelimiter,
     * TileGroup, Metadata, TileList, and Padding OBUs are skipped silently.
     *
     * @param obus the OBUs to scan
     * @return the collected Sequence Headers, Frame Headers, and failures
     */
    public static Av1ObuStream parseAv1ObuStream(List<Obu> obus) {
        return nParseAv1ObuStream(obus);
    }
}
