package org.tstrans.codec;

import java.nio.ByteBuffer;

/**
 * Parsed H.265 / HEVC Sequence Parameter Set.
 * Mirrors {@code tst_core::codec::h265::H265Sps} (and tst-py's
 * {@code tstrans.codec.H265Sps}).
 *
 * <p>Constructed natively via {@link Builder} (the native parser sets each field
 * by name, matching the H.264 / KLV marshalling precedent). The
 * {@code width}/{@code height}/crop fields are Java {@code long} because the
 * underlying Rust fields are {@code u32}; {@code generalProfileCompatibilityFlags}
 * is {@code long} for the same reason.
 *
 * @param spsSeqParameterSetId            {@code sps_seq_parameter_set_id} (H.265 §7.4.3.2.1)
 * @param spsVideoParameterSetId          {@code sps_video_parameter_set_id} linking to a VPS
 * @param width                           post-crop display width in luma samples
 * @param height                          post-crop display height in luma samples
 * @param generalProfileIdc               {@code general_profile_idc} (1=Main, 2=Main10, …)
 * @param generalTierFlag                 {@code general_tier_flag} (true = High tier)
 * @param generalLevelIdc                 {@code general_level_idc} — e.g. 120 for Level 4.0
 * @param generalProfileCompatibilityFlags 32-bit {@code general_profile_compatibility_flags}
 *                                        (§7.3.3) — see {@link H265ProfileTierLevel}
 * @param generalProgressiveSourceFlag    {@code general_progressive_source_flag} (§7.4.4)
 * @param generalInterlacedSourceFlag     {@code general_interlaced_source_flag} (§7.4.4)
 * @param generalNonPackedConstraintFlag  {@code general_non_packed_constraint_flag} (§7.4.4)
 * @param generalFrameOnlyConstraintFlag  {@code general_frame_only_constraint_flag} (§7.4.4)
 * @param bitDepthLuma                    luma bit depth (8 + {@code bit_depth_luma_minus8})
 * @param bitDepthChroma                  chroma bit depth (8 + {@code bit_depth_chroma_minus8})
 * @param chromaFormat                    chroma subsampling format
 * @param maxSubLayersMinus1              {@code sps_max_sub_layers_minus1}
 * @param frameRate                       frame rate as {@link Rational}, or {@code null} when no VUI timing
 * @param color                           VUI colour info, or {@code null} when absent
 * @param cropLeft                        left conformance-window crop in luma samples (§7.4.3.2.1)
 * @param cropRight                       right conformance-window crop in luma samples
 * @param cropTop                         top conformance-window crop in luma samples
 * @param cropBottom                      bottom conformance-window crop in luma samples
 * @param log2MaxPicOrderCntLsbMinus4     {@code log2_max_pic_order_cnt_lsb_minus4} —
 *                                        {@code pic_order_cnt_lsb} width = this + 4
 * @param rawRbsp                         original RBSP bytes (heap {@code ByteBuffer})
 */
public record H265Sps(
        int spsSeqParameterSetId,
        int spsVideoParameterSetId,
        long width,
        long height,
        int generalProfileIdc,
        boolean generalTierFlag,
        int generalLevelIdc,
        long generalProfileCompatibilityFlags,
        boolean generalProgressiveSourceFlag,
        boolean generalInterlacedSourceFlag,
        boolean generalNonPackedConstraintFlag,
        boolean generalFrameOnlyConstraintFlag,
        int bitDepthLuma,
        int bitDepthChroma,
        ChromaFormat chromaFormat,
        int maxSubLayersMinus1,
        Rational frameRate,
        ColorInfo color,
        long cropLeft,
        long cropRight,
        long cropTop,
        long cropBottom,
        int log2MaxPicOrderCntLsbMinus4,
        ByteBuffer rawRbsp) {

    /**
     * Coded picture width before conformance-window crop is applied
     * (luma samples). Equal to {@code width + cropLeft + cropRight}. Mirrors
     * {@code H265Sps::coded_width()}.
     *
     * @return the coded (pre-crop) width
     */
    public long codedWidth() {
        return width + cropLeft + cropRight;
    }

    /**
     * Coded picture height before conformance-window crop is applied
     * (luma samples). Equal to {@code height + cropTop + cropBottom}. Mirrors
     * {@code H265Sps::coded_height()}.
     *
     * @return the coded (pre-crop) height
     */
    public long codedHeight() {
        return height + cropTop + cropBottom;
    }

    /**
     * Reconstruct the {@code profile_tier_level()} block (§7.3.3) from the PTL
     * fields flattened onto this SPS. Mirrors tst-py's
     * {@code H265Sps.profile_tier_level()} — {@code generalProfileSpace} is
     * {@code 0} (not stored on the SPS; 0 for all ITU-T registered profiles).
     *
     * @return the reconstructed profile-tier-level
     */
    public H265ProfileTierLevel profileTierLevel() {
        return new H265ProfileTierLevel(
                0,
                generalTierFlag,
                generalProfileIdc,
                generalProfileCompatibilityFlags,
                generalProgressiveSourceFlag,
                generalInterlacedSourceFlag,
                generalNonPackedConstraintFlag,
                generalFrameOnlyConstraintFlag,
                generalLevelIdc);
    }

    /**
     * Mutable builder used by the native parser to assemble an {@link H265Sps}
     * field-by-field. Mirrors the H.264 / KLV marshalling precedent — the JNI
     * side invokes the setters then {@link #build()}.
     */
    public static final class Builder {
        private int spsSeqParameterSetId;
        private int spsVideoParameterSetId;
        private long width;
        private long height;
        private int generalProfileIdc;
        private boolean generalTierFlag;
        private int generalLevelIdc;
        private long generalProfileCompatibilityFlags;
        private boolean generalProgressiveSourceFlag;
        private boolean generalInterlacedSourceFlag;
        private boolean generalNonPackedConstraintFlag;
        private boolean generalFrameOnlyConstraintFlag;
        private int bitDepthLuma;
        private int bitDepthChroma;
        private ChromaFormat chromaFormat;
        private int maxSubLayersMinus1;
        private Rational frameRate;
        private ColorInfo color;
        private long cropLeft;
        private long cropRight;
        private long cropTop;
        private long cropBottom;
        private int log2MaxPicOrderCntLsbMinus4;
        private ByteBuffer rawRbsp;

        /** @param v {@code sps_seq_parameter_set_id} @return this builder */
        public Builder spsSeqParameterSetId(int v) {
            this.spsSeqParameterSetId = v;
            return this;
        }

        /** @param v {@code sps_video_parameter_set_id} @return this builder */
        public Builder spsVideoParameterSetId(int v) {
            this.spsVideoParameterSetId = v;
            return this;
        }

        /** @param v post-crop width @return this builder */
        public Builder width(long v) {
            this.width = v;
            return this;
        }

        /** @param v post-crop height @return this builder */
        public Builder height(long v) {
            this.height = v;
            return this;
        }

        /** @param v {@code general_profile_idc} @return this builder */
        public Builder generalProfileIdc(int v) {
            this.generalProfileIdc = v;
            return this;
        }

        /** @param v {@code general_tier_flag} @return this builder */
        public Builder generalTierFlag(boolean v) {
            this.generalTierFlag = v;
            return this;
        }

        /** @param v {@code general_level_idc} @return this builder */
        public Builder generalLevelIdc(int v) {
            this.generalLevelIdc = v;
            return this;
        }

        /** @param v {@code general_profile_compatibility_flags} @return this builder */
        public Builder generalProfileCompatibilityFlags(long v) {
            this.generalProfileCompatibilityFlags = v;
            return this;
        }

        /** @param v {@code general_progressive_source_flag} @return this builder */
        public Builder generalProgressiveSourceFlag(boolean v) {
            this.generalProgressiveSourceFlag = v;
            return this;
        }

        /** @param v {@code general_interlaced_source_flag} @return this builder */
        public Builder generalInterlacedSourceFlag(boolean v) {
            this.generalInterlacedSourceFlag = v;
            return this;
        }

        /** @param v {@code general_non_packed_constraint_flag} @return this builder */
        public Builder generalNonPackedConstraintFlag(boolean v) {
            this.generalNonPackedConstraintFlag = v;
            return this;
        }

        /** @param v {@code general_frame_only_constraint_flag} @return this builder */
        public Builder generalFrameOnlyConstraintFlag(boolean v) {
            this.generalFrameOnlyConstraintFlag = v;
            return this;
        }

        /** @param v luma bit depth @return this builder */
        public Builder bitDepthLuma(int v) {
            this.bitDepthLuma = v;
            return this;
        }

        /** @param v chroma bit depth @return this builder */
        public Builder bitDepthChroma(int v) {
            this.bitDepthChroma = v;
            return this;
        }

        /** @param v chroma subsampling format @return this builder */
        public Builder chromaFormat(ChromaFormat v) {
            this.chromaFormat = v;
            return this;
        }

        /** @param v {@code sps_max_sub_layers_minus1} @return this builder */
        public Builder maxSubLayersMinus1(int v) {
            this.maxSubLayersMinus1 = v;
            return this;
        }

        /** @param v frame rate, or {@code null} @return this builder */
        public Builder frameRate(Rational v) {
            this.frameRate = v;
            return this;
        }

        /** @param v colour info, or {@code null} @return this builder */
        public Builder color(ColorInfo v) {
            this.color = v;
            return this;
        }

        /** @param v left crop offset @return this builder */
        public Builder cropLeft(long v) {
            this.cropLeft = v;
            return this;
        }

        /** @param v right crop offset @return this builder */
        public Builder cropRight(long v) {
            this.cropRight = v;
            return this;
        }

        /** @param v top crop offset @return this builder */
        public Builder cropTop(long v) {
            this.cropTop = v;
            return this;
        }

        /** @param v bottom crop offset @return this builder */
        public Builder cropBottom(long v) {
            this.cropBottom = v;
            return this;
        }

        /** @param v {@code log2_max_pic_order_cnt_lsb_minus4} @return this builder */
        public Builder log2MaxPicOrderCntLsbMinus4(int v) {
            this.log2MaxPicOrderCntLsbMinus4 = v;
            return this;
        }

        /** @param v raw RBSP buffer @return this builder */
        public Builder rawRbsp(ByteBuffer v) {
            this.rawRbsp = v;
            return this;
        }

        /** @return the assembled {@link H265Sps} */
        public H265Sps build() {
            return new H265Sps(
                    spsSeqParameterSetId, spsVideoParameterSetId, width, height,
                    generalProfileIdc, generalTierFlag, generalLevelIdc,
                    generalProfileCompatibilityFlags, generalProgressiveSourceFlag,
                    generalInterlacedSourceFlag, generalNonPackedConstraintFlag,
                    generalFrameOnlyConstraintFlag, bitDepthLuma, bitDepthChroma,
                    chromaFormat, maxSubLayersMinus1, frameRate, color,
                    cropLeft, cropRight, cropTop, cropBottom,
                    log2MaxPicOrderCntLsbMinus4, rawRbsp);
        }
    }
}
