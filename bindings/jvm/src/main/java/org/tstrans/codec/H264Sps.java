package org.tstrans.codec;

import java.nio.ByteBuffer;

/**
 * Parsed H.264 / AVC Sequence Parameter Set.
 * Mirrors {@code tst_core::codec::h264::H264Sps} (and tst-py's
 * {@code tstrans.codec.H264Sps}).
 *
 * <p>Constructed natively via {@link Builder} (the native parser sets each
 * field by name, matching the KLV-set marshalling precedent). The
 * {@code width}/{@code height}/crop fields are Java {@code long} because the
 * underlying Rust fields are {@code u32} (which do not fit a Java {@code int}
 * unsigned).
 *
 * @param seqParameterSetId      {@code seq_parameter_set_id} (H.264 §7.4.2.1.1)
 * @param width                  post-crop display width in luma samples
 * @param height                 post-crop display height in luma samples
 * @param profileIdc             {@code profile_idc} (66=Baseline, 77=Main, 100=High, …)
 * @param levelIdc               {@code level_idc} — e.g. 40 for Level 4.0
 * @param constraintSetFlags     {@code constraint_set_flags} byte (bits 7-2 flags; 1-0 reserved)
 * @param bitDepthLuma           luma bit depth (8 + {@code bit_depth_luma_minus8})
 * @param bitDepthChroma         chroma bit depth (8 + {@code bit_depth_chroma_minus8})
 * @param chromaFormat           chroma subsampling format
 * @param frameMbsOnly           {@code true} for progressive ({@code frame_mbs_only_flag=1})
 * @param fixedFrameRate         {@code true} when {@code fixed_frame_rate_flag=1} in the VUI
 * @param hasBFrames             {@code true} when the stream may contain B-frames (heuristic)
 * @param frameRate              frame rate as {@link Rational}, or {@code null} when no VUI
 * @param color                  VUI colour info, or {@code null} when absent
 * @param cropLeft               left crop offset in luma samples (H.264 §6.4 after scaling)
 * @param cropRight              right crop offset in luma samples
 * @param cropTop                top crop offset in luma samples
 * @param cropBottom             bottom crop offset in luma samples
 * @param log2MaxFrameNumMinus4  {@code log2_max_frame_num_minus4} — {@code frame_num} width = this + 4
 * @param rawRbsp                original RBSP bytes (heap {@code ByteBuffer})
 */
public record H264Sps(
        int seqParameterSetId,
        long width,
        long height,
        int profileIdc,
        int levelIdc,
        int constraintSetFlags,
        int bitDepthLuma,
        int bitDepthChroma,
        ChromaFormat chromaFormat,
        boolean frameMbsOnly,
        boolean fixedFrameRate,
        boolean hasBFrames,
        Rational frameRate,
        ColorInfo color,
        long cropLeft,
        long cropRight,
        long cropTop,
        long cropBottom,
        int log2MaxFrameNumMinus4,
        ByteBuffer rawRbsp) {

    /**
     * Coded picture width before {@code frame_crop} is applied (luma samples).
     * Equal to {@code width + cropLeft + cropRight}. Mirrors
     * {@code H264Sps::coded_width()}.
     *
     * @return the coded (pre-crop) width
     */
    public long codedWidth() {
        return width + cropLeft + cropRight;
    }

    /**
     * Coded picture height before {@code frame_crop} is applied (luma samples).
     * Equal to {@code height + cropTop + cropBottom}. Mirrors
     * {@code H264Sps::coded_height()}.
     *
     * @return the coded (pre-crop) height
     */
    public long codedHeight() {
        return height + cropTop + cropBottom;
    }

    /**
     * Mutable builder used by the native parser to assemble an {@link H264Sps}
     * field-by-field. Mirrors the KLV {@code UasDatalinkLs.Builder} marshalling
     * precedent — the JNI side invokes the setters then {@link #build()}.
     */
    public static final class Builder {
        private int seqParameterSetId;
        private long width;
        private long height;
        private int profileIdc;
        private int levelIdc;
        private int constraintSetFlags;
        private int bitDepthLuma;
        private int bitDepthChroma;
        private ChromaFormat chromaFormat;
        private boolean frameMbsOnly;
        private boolean fixedFrameRate;
        private boolean hasBFrames;
        private Rational frameRate;
        private ColorInfo color;
        private long cropLeft;
        private long cropRight;
        private long cropTop;
        private long cropBottom;
        private int log2MaxFrameNumMinus4;
        private ByteBuffer rawRbsp;

        /** @param v {@code seq_parameter_set_id} @return this builder */
        public Builder seqParameterSetId(int v) {
            this.seqParameterSetId = v;
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

        /** @param v {@code profile_idc} @return this builder */
        public Builder profileIdc(int v) {
            this.profileIdc = v;
            return this;
        }

        /** @param v {@code level_idc} @return this builder */
        public Builder levelIdc(int v) {
            this.levelIdc = v;
            return this;
        }

        /** @param v {@code constraint_set_flags} @return this builder */
        public Builder constraintSetFlags(int v) {
            this.constraintSetFlags = v;
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

        /** @param v progressive flag @return this builder */
        public Builder frameMbsOnly(boolean v) {
            this.frameMbsOnly = v;
            return this;
        }

        /** @param v fixed-frame-rate flag @return this builder */
        public Builder fixedFrameRate(boolean v) {
            this.fixedFrameRate = v;
            return this;
        }

        /** @param v has-B-frames flag @return this builder */
        public Builder hasBFrames(boolean v) {
            this.hasBFrames = v;
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

        /** @param v {@code log2_max_frame_num_minus4} @return this builder */
        public Builder log2MaxFrameNumMinus4(int v) {
            this.log2MaxFrameNumMinus4 = v;
            return this;
        }

        /** @param v raw RBSP buffer @return this builder */
        public Builder rawRbsp(ByteBuffer v) {
            this.rawRbsp = v;
            return this;
        }

        /** @return the assembled {@link H264Sps} */
        public H264Sps build() {
            return new H264Sps(
                    seqParameterSetId, width, height, profileIdc, levelIdc,
                    constraintSetFlags, bitDepthLuma, bitDepthChroma, chromaFormat,
                    frameMbsOnly, fixedFrameRate, hasBFrames, frameRate, color,
                    cropLeft, cropRight, cropTop, cropBottom, log2MaxFrameNumMinus4,
                    rawRbsp);
        }
    }
}
