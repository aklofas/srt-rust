//! H.266 Profile/Tier/Level parser. Per H.266 V4 §7.3.3.

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct H266ProfileTierLevel {
    /// `general_profile_idc` u(7) — H.266 V4 Annex A profile assignment.
    pub general_profile_idc: u8,
    /// `general_tier_flag` u(1) — 0 = Main tier, 1 = High tier.
    pub general_tier_flag: bool,
    /// `general_level_idc` u(8) — H.266 V4 Annex A.4 level table.
    pub general_level_idc: u8,
}
