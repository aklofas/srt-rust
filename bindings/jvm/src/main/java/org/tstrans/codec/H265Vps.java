package org.tstrans.codec;

import java.nio.ByteBuffer;

/**
 * Parsed H.265 / HEVC Video Parameter Set.
 * Mirrors {@code tst_core::codec::h265::H265Vps} (and tst-py's
 * {@code tstrans.codec.H265Vps}).
 *
 * <p>Constructed natively via {@link Builder}. Only the fields decoded up
 * through {@code profile_tier_level()} are surfaced; everything past
 * {@code general_level_idc} is skipped. {@code generalProfileCompatibilityFlags}
 * is Java {@code long} because the underlying Rust field is {@code u32}.
 *
 * @param vpsVideoParameterSetId          {@code vps_video_parameter_set_id} (H.265 §7.4.3.1)
 * @param maxLayersMinus1                 {@code vps_max_layers_minus1}
 * @param maxSubLayersMinus1              {@code vps_max_sub_layers_minus1}
 * @param temporalIdNestingFlag           {@code vps_temporal_id_nesting_flag}
 * @param generalProfileIdc               {@code general_profile_idc} (1=Main, 2=Main10, …)
 * @param generalTierFlag                 {@code general_tier_flag} (true = High tier)
 * @param generalLevelIdc                 {@code general_level_idc} — e.g. 120 for Level 4.0
 * @param generalProfileCompatibilityFlags 32-bit {@code general_profile_compatibility_flags}
 *                                        (§7.3.3) — see {@link H265ProfileTierLevel}
 * @param generalProgressiveSourceFlag    {@code general_progressive_source_flag} (§7.4.4)
 * @param generalInterlacedSourceFlag     {@code general_interlaced_source_flag} (§7.4.4)
 * @param generalNonPackedConstraintFlag  {@code general_non_packed_constraint_flag} (§7.4.4)
 * @param generalFrameOnlyConstraintFlag  {@code general_frame_only_constraint_flag} (§7.4.4)
 * @param rawRbsp                         original RBSP bytes (heap {@code ByteBuffer})
 */
public record H265Vps(
        int vpsVideoParameterSetId,
        int maxLayersMinus1,
        int maxSubLayersMinus1,
        boolean temporalIdNestingFlag,
        int generalProfileIdc,
        boolean generalTierFlag,
        int generalLevelIdc,
        long generalProfileCompatibilityFlags,
        boolean generalProgressiveSourceFlag,
        boolean generalInterlacedSourceFlag,
        boolean generalNonPackedConstraintFlag,
        boolean generalFrameOnlyConstraintFlag,
        ByteBuffer rawRbsp) {

    /**
     * Reconstruct the {@code profile_tier_level()} block (§7.3.3) from the PTL
     * fields flattened onto this VPS. Mirrors tst-py's
     * {@code H265Vps.profile_tier_level()} — {@code generalProfileSpace} is
     * {@code 0} (not stored on the VPS; 0 for all ITU-T registered profiles).
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
     * Mutable builder used by the native parser to assemble an {@link H265Vps}
     * field-by-field.
     */
    public static final class Builder {
        private int vpsVideoParameterSetId;
        private int maxLayersMinus1;
        private int maxSubLayersMinus1;
        private boolean temporalIdNestingFlag;
        private int generalProfileIdc;
        private boolean generalTierFlag;
        private int generalLevelIdc;
        private long generalProfileCompatibilityFlags;
        private boolean generalProgressiveSourceFlag;
        private boolean generalInterlacedSourceFlag;
        private boolean generalNonPackedConstraintFlag;
        private boolean generalFrameOnlyConstraintFlag;
        private ByteBuffer rawRbsp;

        /** @param v {@code vps_video_parameter_set_id} @return this builder */
        public Builder vpsVideoParameterSetId(int v) {
            this.vpsVideoParameterSetId = v;
            return this;
        }

        /** @param v {@code vps_max_layers_minus1} @return this builder */
        public Builder maxLayersMinus1(int v) {
            this.maxLayersMinus1 = v;
            return this;
        }

        /** @param v {@code vps_max_sub_layers_minus1} @return this builder */
        public Builder maxSubLayersMinus1(int v) {
            this.maxSubLayersMinus1 = v;
            return this;
        }

        /** @param v {@code vps_temporal_id_nesting_flag} @return this builder */
        public Builder temporalIdNestingFlag(boolean v) {
            this.temporalIdNestingFlag = v;
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

        /** @param v raw RBSP buffer @return this builder */
        public Builder rawRbsp(ByteBuffer v) {
            this.rawRbsp = v;
            return this;
        }

        /** @return the assembled {@link H265Vps} */
        public H265Vps build() {
            return new H265Vps(
                    vpsVideoParameterSetId, maxLayersMinus1, maxSubLayersMinus1,
                    temporalIdNestingFlag, generalProfileIdc, generalTierFlag,
                    generalLevelIdc, generalProfileCompatibilityFlags,
                    generalProgressiveSourceFlag, generalInterlacedSourceFlag,
                    generalNonPackedConstraintFlag, generalFrameOnlyConstraintFlag,
                    rawRbsp);
        }
    }
}
