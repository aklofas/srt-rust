package org.tstrans.codec;

/**
 * Decoded {@code profile_tier_level()} fields (H.265 §7.3.3).
 * Mirrors {@code tst_core::codec::h265::H265ProfileTierLevel} (and tst-py's
 * {@code tstrans.codec.H265ProfileTierLevel}).
 *
 * <p>Both {@link H265Sps#profileTierLevel()} and {@link H265Vps#profileTierLevel()}
 * reconstruct this from the PTL fields flattened onto the SPS / VPS.
 *
 * <p>{@code generalProfileSpace} is always {@code 0} when reconstructed from an
 * SPS or VPS — neither {@link H265Sps} nor {@link H265Vps} stores that field (it
 * is {@code 0} for all ITU-T registered profiles). This matches tst-py's
 * {@code ptl_from_sps_fields}, which hardcodes {@code 0}.
 *
 * @param generalProfileSpace             {@code general_profile_space} (always 0 here)
 * @param generalTierFlag                 {@code general_tier_flag} (true = High tier)
 * @param generalProfileIdc               {@code general_profile_idc} (1=Main, 2=Main10, …)
 * @param generalProfileCompatibilityFlags 32-bit {@code general_profile_compatibility_flags}
 *                                         (§7.3.3) — Java {@code long} because the Rust field is {@code u32}
 * @param generalProgressiveSourceFlag    {@code general_progressive_source_flag} (§7.4.4)
 * @param generalInterlacedSourceFlag     {@code general_interlaced_source_flag} (§7.4.4)
 * @param generalNonPackedConstraintFlag  {@code general_non_packed_constraint_flag} (§7.4.4)
 * @param generalFrameOnlyConstraintFlag  {@code general_frame_only_constraint_flag} (§7.4.4)
 * @param generalLevelIdc                 {@code general_level_idc} — e.g. 120 for Level 4.0
 */
public record H265ProfileTierLevel(
        int generalProfileSpace,
        boolean generalTierFlag,
        int generalProfileIdc,
        long generalProfileCompatibilityFlags,
        boolean generalProgressiveSourceFlag,
        boolean generalInterlacedSourceFlag,
        boolean generalNonPackedConstraintFlag,
        boolean generalFrameOnlyConstraintFlag,
        int generalLevelIdc) {
}
