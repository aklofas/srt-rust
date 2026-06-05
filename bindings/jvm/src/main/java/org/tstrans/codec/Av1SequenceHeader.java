package org.tstrans.codec;

import java.nio.ByteBuffer;

/**
 * Parsed AV1 Sequence Header OBU (AV1 Bitstream Spec §5.5).
 * Mirrors {@code tst_core::codec::av1::Av1SequenceHeader} (and tst-py's
 * {@code tstrans.codec.Av1SequenceHeader}).
 *
 * <p>Constructed natively via {@link Builder} (the native parser sets each field
 * by name, matching the H.264 / H.265 / H.266 marshalling precedent). The
 * {@code maxFrameWidth}/{@code maxFrameHeight} fields are Java {@code long}
 * because the underlying Rust fields are {@code u32}.
 *
 * @param profile                   {@code seq_profile} — 0=Main, 1=High,
 *                                  2=Professional
 * @param level                     {@code seq_level_idx[0]} — operating point 0
 *                                  level index
 * @param tier                      {@code seq_tier[0]} — operating point 0 tier
 *                                  (0 unless level > 7)
 * @param maxFrameWidth             {@code max_frame_width_minus_1 + 1}
 * @param maxFrameHeight            {@code max_frame_height_minus_1 + 1}
 * @param bitDepth                  8, 10, or 12 per {@code BitDepth} derivation
 *                                  in §5.5.2
 * @param monochrome                true when {@code mono_chrome = 1} (Y-only)
 * @param chromaFormat              chroma subsampling format
 * @param stillPicture              true when {@code still_picture = 1}
 * @param reducedStillPictureHeader true when
 *                                  {@code reduced_still_picture_header = 1}
 * @param colorInfo                 colour metadata, or {@code null}. AV1 always
 *                                  carries a {@code color_range} bit, so a
 *                                  successful parse populates at least the
 *                                  dynamic-range signal; the {@code null} case
 *                                  is reserved for forward-compatibility
 * @param frameRate                 frame rate as {@link Rational}, derived from
 *                                  {@code time_scale / num_units_in_display_tick}
 *                                  when timing info is present, else {@code null}
 * @param raw                       original payload bytes (heap {@code ByteBuffer})
 */
public record Av1SequenceHeader(
        int profile,
        int level,
        int tier,
        long maxFrameWidth,
        long maxFrameHeight,
        int bitDepth,
        boolean monochrome,
        ChromaFormat chromaFormat,
        boolean stillPicture,
        boolean reducedStillPictureHeader,
        ColorInfo colorInfo,
        Rational frameRate,
        ByteBuffer raw) {

    /**
     * Mutable builder used by the native parser to assemble an
     * {@link Av1SequenceHeader} field-by-field. Mirrors the H.26x marshalling
     * precedent — the JNI side invokes the setters then {@link #build()}.
     */
    public static final class Builder {
        private int profile;
        private int level;
        private int tier;
        private long maxFrameWidth;
        private long maxFrameHeight;
        private int bitDepth;
        private boolean monochrome;
        private ChromaFormat chromaFormat;
        private boolean stillPicture;
        private boolean reducedStillPictureHeader;
        private ColorInfo colorInfo;
        private Rational frameRate;
        private ByteBuffer raw;

        /** @param v {@code seq_profile} @return this builder */
        public Builder profile(int v) {
            this.profile = v;
            return this;
        }

        /** @param v {@code seq_level_idx[0]} @return this builder */
        public Builder level(int v) {
            this.level = v;
            return this;
        }

        /** @param v {@code seq_tier[0]} @return this builder */
        public Builder tier(int v) {
            this.tier = v;
            return this;
        }

        /** @param v {@code max_frame_width_minus_1 + 1} @return this builder */
        public Builder maxFrameWidth(long v) {
            this.maxFrameWidth = v;
            return this;
        }

        /** @param v {@code max_frame_height_minus_1 + 1} @return this builder */
        public Builder maxFrameHeight(long v) {
            this.maxFrameHeight = v;
            return this;
        }

        /** @param v {@code BitDepth} (8/10/12) @return this builder */
        public Builder bitDepth(int v) {
            this.bitDepth = v;
            return this;
        }

        /** @param v {@code mono_chrome} flag @return this builder */
        public Builder monochrome(boolean v) {
            this.monochrome = v;
            return this;
        }

        /** @param v chroma subsampling format @return this builder */
        public Builder chromaFormat(ChromaFormat v) {
            this.chromaFormat = v;
            return this;
        }

        /** @param v {@code still_picture} flag @return this builder */
        public Builder stillPicture(boolean v) {
            this.stillPicture = v;
            return this;
        }

        /** @param v {@code reduced_still_picture_header} flag @return this builder */
        public Builder reducedStillPictureHeader(boolean v) {
            this.reducedStillPictureHeader = v;
            return this;
        }

        /** @param v colour info, or {@code null} @return this builder */
        public Builder colorInfo(ColorInfo v) {
            this.colorInfo = v;
            return this;
        }

        /** @param v frame rate, or {@code null} @return this builder */
        public Builder frameRate(Rational v) {
            this.frameRate = v;
            return this;
        }

        /** @param v raw payload buffer @return this builder */
        public Builder raw(ByteBuffer v) {
            this.raw = v;
            return this;
        }

        /** @return the assembled {@link Av1SequenceHeader} */
        public Av1SequenceHeader build() {
            return new Av1SequenceHeader(
                    profile, level, tier, maxFrameWidth, maxFrameHeight, bitDepth,
                    monochrome, chromaFormat, stillPicture, reducedStillPictureHeader,
                    colorInfo, frameRate, raw);
        }
    }
}
