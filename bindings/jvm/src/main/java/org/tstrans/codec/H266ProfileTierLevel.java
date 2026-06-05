package org.tstrans.codec;

/**
 * Decoded {@code profile_tier_level()} headline fields (H.266 V4 §7.3.3).
 * Mirrors {@code tst_core::codec::h266::H266ProfileTierLevel} (and tst-py's
 * {@code tstrans.codec.H266ProfileTierLevel}).
 *
 * <p>H.266 PTL carries far fewer surfaced fields than H.265 — only the three
 * headline values. Unlike H.265 (where the PTL is reconstructed from fields
 * flattened onto the SPS/VPS), H.266 stores this as a real nested sub-record on
 * {@link H266Sps#profileTierLevel()}.
 *
 * @param generalProfileIdc 7-bit {@code general_profile_idc} (1=Main10,
 *                          2=MultilayerMain10 per H.266 V4 Annex A)
 * @param generalTierFlag   {@code general_tier_flag} — false = Main tier,
 *                          true = High tier
 * @param generalLevelIdc   {@code general_level_idc} — H.266 V4 Annex A.4 level
 *                          table (e.g. 64 = Level 4.0, encoded as level × 16)
 */
public record H266ProfileTierLevel(
        int generalProfileIdc,
        boolean generalTierFlag,
        int generalLevelIdc) {
}
