package org.tstrans.codec;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNotNull;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertSame;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.util.List;
import org.junit.jupiter.api.Test;
import org.tstrans.CodecParseException;

class H264Test {
    // Real 1080p High@4.0 BT.709 SPS — the exact bytes the tst_core Rust unit
    // test `parse_sps_1080p_high_dimensions` feeds (fixture
    // crates/tst-core/tests/fixtures/codec/h264/h264_1080p_high40_bt709_sps.bin).
    // Asserted dimensions/profile/level/crop reused verbatim.
    private static final byte[] SPS_1080P_HIGH40 = unsigned(
            100, 16, 40, 172, 184, 15, 0, 68, 252, 184, 11, 80, 16, 16, 20, 0,
            3, 0, 4, 0, 0, 3, 0, 240, 16);

    // Real 1080p High@4.0 PPS (same fixture set).
    private static final byte[] PPS_1080P_HIGH40 = unsigned(238, 15, 44, 139);

    // Synthetic minimal IDR slice header, no SPS context — the exact bytes the
    // Rust slice_header_light test `synth_idr_slice_header_no_sps` feeds:
    // first_mb_in_slice=0, slice_type=7 (mod 5 = 2 = I), pps_id=0.
    private static final byte[] IDR_SLICE_NO_SPS = unsigned(0x88, 0x80);

    private static byte[] unsigned(int... vals) {
        byte[] out = new byte[vals.length];
        for (int i = 0; i < vals.length; i++) {
            out[i] = (byte) vals[i];
        }
        return out;
    }

    @Test
    void parseSps1080pHighDimensions() throws CodecParseException {
        H264Sps sps = Codec.parseH264Sps(SPS_1080P_HIGH40);
        assertEquals(1920L, sps.width());
        assertEquals(1080L, sps.height());
        assertEquals(100, sps.profileIdc());
        assertEquals(40, sps.levelIdc());
        assertEquals(8, sps.bitDepthLuma());
        assertEquals(8, sps.bitDepthChroma());
        assertSame(ChromaFormat.YUV420, sps.chromaFormat());
        assertTrue(sps.frameMbsOnly());
        assertEquals(0, sps.seqParameterSetId());
        // Coded dims: 1920x1088 with an 8-luma-sample bottom crop.
        assertEquals(1920L, sps.codedWidth());
        assertEquals(1088L, sps.codedHeight());
        assertEquals(0L, sps.cropLeft());
        assertEquals(0L, sps.cropRight());
        assertEquals(0L, sps.cropTop());
        assertEquals(8L, sps.cropBottom());
        // BT.709 colour signalled in the VUI.
        assertNotNull(sps.color());
        assertSame(ColourPrimaries.BT709, sps.color().primaries());
        assertSame(TransferCharacteristics.BT709, sps.color().transfer());
        assertSame(MatrixCoefficients.BT709, sps.color().matrix());
        // raw_rbsp is preserved byte-for-byte.
        assertEquals(SPS_1080P_HIGH40.length, sps.rawRbsp().remaining());
    }

    @Test
    void parseSpsEmptyThrowsTruncated() {
        CodecParseException ex = assertThrows(
                CodecParseException.class, () -> Codec.parseH264Sps(new byte[0]));
        assertSame(CodecParseException.Kind.TRUNCATED_RBSP, ex.kind());
        assertEquals("h264", ex.codec());
    }

    @Test
    void parsePps1080pHighBasics() throws CodecParseException {
        H264Pps pps = Codec.parseH264Pps(PPS_1080P_HIGH40);
        assertEquals(0, pps.picParameterSetId());
        assertEquals(0, pps.seqParameterSetId());
        assertNotNull(pps.entropyCodingMode());
        assertEquals(PPS_1080P_HIGH40.length, pps.rawRbsp().remaining());
    }

    @Test
    void parsePpsEmptyThrows() {
        assertThrows(CodecParseException.class, () -> Codec.parseH264Pps(new byte[0]));
    }

    @Test
    void parseSliceHeaderLightIdrNoSps() throws CodecParseException {
        H264SliceHeaderLight h = Codec.parseH264SliceHeaderLight(IDR_SLICE_NO_SPS, null, 5);
        assertTrue(h.firstInPic());
        assertSame(H264SliceType.I, h.sliceType());
        assertEquals(0, h.ppsId());
        assertNull(h.frameNum());
        assertTrue(h.idr());
    }

    @Test
    void parseSliceHeaderLightNonIdrMarksIdrFalse() throws CodecParseException {
        H264SliceHeaderLight h = Codec.parseH264SliceHeaderLight(IDR_SLICE_NO_SPS, null, 1);
        assertFalse(h.idr());
    }

    @Test
    void parseSliceHeaderLightWithSpsReadsFrameNum() throws CodecParseException {
        // With SPS context, frame_num is read (bit width = log2MaxFrameNumMinus4 + 4).
        H264Sps sps = Codec.parseH264Sps(SPS_1080P_HIGH40);
        H264SliceHeaderLight h = Codec.parseH264SliceHeaderLight(IDR_SLICE_NO_SPS, sps, 5);
        // The 1080p SPS has log2_max_frame_num_minus4=0 → frame_num is u(4); the
        // 4 bits following the 9-bit header in {0x88,0x80} are 0000 → frame_num 0.
        assertEquals(Integer.valueOf(0), h.frameNum());
    }

    @Test
    void parseSliceHeaderLightEmptyThrows() {
        assertThrows(
                CodecParseException.class,
                () -> Codec.parseH264SliceHeaderLight(new byte[0], null, 1));
    }

    @Test
    void parseParameterSetsSpsPlusPps() throws CodecParseException {
        List<NalUnit> nals = List.of(
                NalUnit.h264(7, 3, SPS_1080P_HIGH40),
                NalUnit.h264(8, 3, PPS_1080P_HIGH40));
        H264ParameterSets ps = Codec.parseH264ParameterSets(nals);
        assertEquals(1, ps.spsById().size());
        assertEquals(1, ps.ppsById().size());
        assertEquals(1920L, ps.spsById().get(0).width());
        assertEquals(0, ps.ppsById().get(0).seqParameterSetId());
    }

    @Test
    void parseParameterSetsSkipsNonParamNals() throws CodecParseException {
        // A slice NAL (type 5) is silently skipped; only the SPS/PPS land.
        byte[] slice = new byte[32];
        for (int i = 0; i < slice.length; i++) {
            slice[i] = (byte) 0xff;
        }
        List<NalUnit> nals = List.of(
                NalUnit.h264(5, 3, slice),
                NalUnit.h264(7, 3, SPS_1080P_HIGH40),
                NalUnit.h264(8, 3, PPS_1080P_HIGH40));
        H264ParameterSets ps = Codec.parseH264ParameterSets(nals);
        assertEquals(1, ps.spsById().size());
    }
}
