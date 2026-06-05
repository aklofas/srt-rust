package org.tstrans.codec;

import java.nio.ByteBuffer;

/**
 * Parsed H.266 / VVC Sequence Parameter Set.
 * Mirrors {@code tst_core::codec::h266::H266Sps} (and tst-py's
 * {@code tstrans.codec.H266Sps}).
 *
 * <p>Constructed natively via {@link Builder} (the native parser sets each field
 * by name, matching the H.264 / H.265 / KLV marshalling precedent). The
 * {@code width}/{@code height}/crop fields are Java {@code long} because the
 * underlying Rust fields are {@code u32}.
 *
 * <p>Unlike {@link H265Sps} (where the profile-tier-level is reconstructed from
 * flattened fields), H.266 stores {@link #profileTierLevel} as a real nested
 * sub-record — H.266 V4 §7.3.3 PTL carries only the three headline fields.
 *
 * @param spsId            {@code sps_seq_parameter_set_id} (H.266 V4 §7.3.2.4)
 * @param vpsId            {@code sps_video_parameter_set_id} linking to a VPS
 * @param profileTierLevel decoded {@code profile_tier_level()} headline fields
 * @param width            post-crop display width in luma samples
 * @param height           post-crop display height in luma samples
 * @param chromaFormat     chroma subsampling format
 * @param bitDepthLuma     luma bit depth (8 + {@code sps_bitdepth_minus8})
 * @param bitDepthChroma   chroma bit depth — equal to {@code bitDepthLuma}
 *                         per H.266 V4 §7.4.3.4 (single field covers both)
 * @param color            VUI colour info, or {@code null} when absent
 * @param frameRate        frame rate as {@link Rational}, or {@code null} when
 *                         no {@code general_timing_hrd_parameters()} timing
 * @param cropLeft         left conformance-window crop in luma samples (§7.4.3.4)
 * @param cropRight        right conformance-window crop in luma samples
 * @param cropTop          top conformance-window crop in luma samples
 * @param cropBottom       bottom conformance-window crop in luma samples
 * @param rawRbsp          original RBSP bytes (heap {@code ByteBuffer})
 */
public record H266Sps(
        int spsId,
        int vpsId,
        H266ProfileTierLevel profileTierLevel,
        long width,
        long height,
        ChromaFormat chromaFormat,
        int bitDepthLuma,
        int bitDepthChroma,
        ColorInfo color,
        Rational frameRate,
        long cropLeft,
        long cropRight,
        long cropTop,
        long cropBottom,
        ByteBuffer rawRbsp) {

    /**
     * Coded picture width before conformance-window crop is applied
     * (luma samples). Equal to {@code width + cropLeft + cropRight}. Mirrors
     * {@code H266Sps::coded_width()}.
     *
     * @return the coded (pre-crop) width
     */
    public long codedWidth() {
        return width + cropLeft + cropRight;
    }

    /**
     * Coded picture height before conformance-window crop is applied
     * (luma samples). Equal to {@code height + cropTop + cropBottom}. Mirrors
     * {@code H266Sps::coded_height()}.
     *
     * @return the coded (pre-crop) height
     */
    public long codedHeight() {
        return height + cropTop + cropBottom;
    }

    /**
     * {@code general_profile_idc} from {@link #profileTierLevel}. Convenience
     * accessor mirroring tst-py's flattened {@code general_profile_idc} getter.
     *
     * @return the 7-bit profile idc
     */
    public int generalProfileIdc() {
        return profileTierLevel.generalProfileIdc();
    }

    /**
     * {@code general_tier_flag} from {@link #profileTierLevel}.
     *
     * @return false = Main tier, true = High tier
     */
    public boolean generalTierFlag() {
        return profileTierLevel.generalTierFlag();
    }

    /**
     * {@code general_level_idc} from {@link #profileTierLevel}.
     *
     * @return the level idc (H.266 V4 Annex A.4)
     */
    public int generalLevelIdc() {
        return profileTierLevel.generalLevelIdc();
    }

    /**
     * Mutable builder used by the native parser to assemble an {@link H266Sps}
     * field-by-field. Mirrors the H.265 marshalling precedent — the JNI side
     * invokes the setters then {@link #build()}.
     */
    public static final class Builder {
        private int spsId;
        private int vpsId;
        private H266ProfileTierLevel profileTierLevel;
        private long width;
        private long height;
        private ChromaFormat chromaFormat;
        private int bitDepthLuma;
        private int bitDepthChroma;
        private ColorInfo color;
        private Rational frameRate;
        private long cropLeft;
        private long cropRight;
        private long cropTop;
        private long cropBottom;
        private ByteBuffer rawRbsp;

        /** @param v {@code sps_seq_parameter_set_id} @return this builder */
        public Builder spsId(int v) {
            this.spsId = v;
            return this;
        }

        /** @param v {@code sps_video_parameter_set_id} @return this builder */
        public Builder vpsId(int v) {
            this.vpsId = v;
            return this;
        }

        /** @param v the nested profile-tier-level sub-record @return this builder */
        public Builder profileTierLevel(H266ProfileTierLevel v) {
            this.profileTierLevel = v;
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

        /** @param v chroma subsampling format @return this builder */
        public Builder chromaFormat(ChromaFormat v) {
            this.chromaFormat = v;
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

        /** @param v colour info, or {@code null} @return this builder */
        public Builder color(ColorInfo v) {
            this.color = v;
            return this;
        }

        /** @param v frame rate, or {@code null} @return this builder */
        public Builder frameRate(Rational v) {
            this.frameRate = v;
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

        /** @param v raw RBSP buffer @return this builder */
        public Builder rawRbsp(ByteBuffer v) {
            this.rawRbsp = v;
            return this;
        }

        /** @return the assembled {@link H266Sps} */
        public H266Sps build() {
            return new H266Sps(
                    spsId, vpsId, profileTierLevel, width, height, chromaFormat,
                    bitDepthLuma, bitDepthChroma, color, frameRate,
                    cropLeft, cropRight, cropTop, cropBottom, rawRbsp);
        }
    }
}
