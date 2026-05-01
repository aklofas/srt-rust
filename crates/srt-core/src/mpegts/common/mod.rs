//! Shared types for `mpegts::mux` (and eventually `mpegts::demux`).
//!
//! Concrete types only — no traits, no generics. The deferred `mpegts::demux`
//! reuses these from day one to avoid mid-flight refactors. See the design
//! doc for the full deferral rationale.

pub mod crc32;

/// MPEG-TS PMT `stream_type` values used by this library.
///
/// Single enum covers both directions: mux looks up by codec / KLV mode,
/// demux looks up by parsed byte. Variants kept narrow to v0 scope —
/// not a general MPEG-TS type registry.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamType {
    /// H.264 / AVC video (ITU-T H.264 / ISO/IEC 14496-10).
    H264 = 0x1B,
    /// H.265 / HEVC video (ITU-T H.265 / ISO/IEC 23008-2).
    H265 = 0x24,
    /// PES private data, typically used for KLV per ST 1402 async.
    KlvPrivate = 0x06,
    /// Synchronous metadata stream per ST 1402 sync.
    KlvSyncMetadata = 0x15,
}

impl StreamType {
    /// Return the underlying byte value as written in PMT.
    pub fn as_u8(self) -> u8 {
        self as u8
    }
}

/// MPEG-TS descriptor tags relevant to mux output.
pub mod descriptor {
    /// `registration_descriptor` (ISO/IEC 13818-1 §2.6.8).
    pub const REGISTRATION: u8 = 0x05;

    /// 4-byte `format_identifier` for KLV asynchronous data per ST 1402 §5.4.
    pub const KLVA: [u8; 4] = *b"KLVA";
}

/// MPEG-TS PID assignments + helpers.
pub mod pid {
    /// PID 0x0000 — Program Association Table (PAT).
    pub const PAT: u16 = 0x0000;

    /// PID 0x1FFF — null packets (used for stuffing in CBR; we don't emit).
    pub const NULL: u16 = 0x1FFF;

    /// User-program PIDs are 0x0010..=0x1FFE per ISO/IEC 13818-1 §2.4.
    pub fn is_user_pid(pid: u16) -> bool {
        (0x0010..=0x1FFE).contains(&pid)
    }
}

/// 90 kHz timestamp — used for PES PTS/DTS encoding.
///
/// Newtype around `i64` so callers can't accidentally pass milliseconds or
/// 27 MHz values. Spec range is 33-bit unsigned (0..=2^33-1), but we accept
/// `i64` to match what encoders produce; values are masked at encoding time
/// (PES PTS) and at PCR derivation (`Pcr27mhz::from_pts`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Pts90khz(pub i64);

impl Pts90khz {
    /// Convert milliseconds to 90 kHz ticks.
    pub fn from_millis(ms: i64) -> Self {
        Self(ms * 90)
    }

    /// Mask to the 33-bit PES PTS field range.
    pub fn masked_33bit(self) -> u64 {
        (self.0 as u64) & ((1u64 << 33) - 1)
    }
}

/// 27 MHz PCR timestamp — base × 300 + extension per ISO/IEC 13818-1 §2.4.3.5.
///
/// Stored as the full 27 MHz value; encoding splits it into the 33-bit base
/// and 9-bit extension at write time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Pcr27mhz(pub u64);

impl Pcr27mhz {
    /// Build from milliseconds.
    pub fn from_millis(ms: u64) -> Self {
        Self(ms * 27_000)
    }

    /// Build from a 90 kHz PTS (PCR base = PTS, extension = 0).
    pub fn from_pts(pts: Pts90khz) -> Self {
        Self(pts.masked_33bit() * 300)
    }

    /// 33-bit PCR base (90 kHz units).
    pub fn base(self) -> u64 {
        (self.0 / 300) & ((1u64 << 33) - 1)
    }

    /// 9-bit PCR extension (27 MHz units modulo 300).
    pub fn extension(self) -> u16 {
        (self.0 % 300) as u16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_type_byte_values() {
        assert_eq!(StreamType::H264.as_u8(), 0x1B);
        assert_eq!(StreamType::H265.as_u8(), 0x24);
        assert_eq!(StreamType::KlvPrivate.as_u8(), 0x06);
        assert_eq!(StreamType::KlvSyncMetadata.as_u8(), 0x15);
    }

    #[test]
    fn klva_descriptor_bytes() {
        assert_eq!(descriptor::KLVA, [0x4B, 0x4C, 0x56, 0x41]);
        assert_eq!(descriptor::REGISTRATION, 0x05);
    }

    #[test]
    fn user_pid_range() {
        assert!(!pid::is_user_pid(0x0000));
        assert!(!pid::is_user_pid(0x000F));
        assert!(pid::is_user_pid(0x0010));
        assert!(pid::is_user_pid(0x1011));
        assert!(pid::is_user_pid(0x1FFE));
        assert!(!pid::is_user_pid(0x1FFF));
    }

    #[test]
    fn pts_from_millis() {
        assert_eq!(Pts90khz::from_millis(0).0, 0);
        assert_eq!(Pts90khz::from_millis(1000).0, 90_000);
    }

    #[test]
    fn pts_masking() {
        // 33-bit max = 0x1_FFFF_FFFF
        assert_eq!(Pts90khz(0x1_FFFF_FFFF).masked_33bit(), 0x1_FFFF_FFFF);
        // Higher bits get masked off.
        assert_eq!(Pts90khz(0x3_FFFF_FFFF).masked_33bit(), 0x1_FFFF_FFFF);
    }

    #[test]
    fn pcr_from_millis() {
        assert_eq!(Pcr27mhz::from_millis(0).0, 0);
        assert_eq!(Pcr27mhz::from_millis(40).0, 40 * 27_000);
    }

    #[test]
    fn pcr_base_extension_split() {
        // 90 kHz tick * 300 = exactly one base unit, zero extension.
        let pcr = Pcr27mhz::from_pts(Pts90khz(1));
        assert_eq!(pcr.base(), 1);
        assert_eq!(pcr.extension(), 0);

        // Mid-base value: 1 base + 150 extension.
        let pcr = Pcr27mhz(300 + 150);
        assert_eq!(pcr.base(), 1);
        assert_eq!(pcr.extension(), 150);
    }

    #[test]
    fn pcr_base_masks_to_33bit() {
        // Exactly at the 33-bit boundary: bit 33 set, low bits clear.
        // Pre-mask base = 1u64 << 33; post-mask should be 0.
        let pcr = Pcr27mhz((1u64 << 33) * 300);
        assert_eq!(pcr.base(), 0);

        // Bit 33 set plus a low-bit value: the high bit gets stripped, low bits preserved.
        let pcr = Pcr27mhz(((1u64 << 33) + 1) * 300);
        assert_eq!(pcr.base(), 1);

        // Far-above-mask value: only the low 33 bits should survive.
        let pcr = Pcr27mhz((1u64 << 50 | 0x1234_5678) * 300);
        assert_eq!(pcr.base(), 0x1234_5678);
    }
}
