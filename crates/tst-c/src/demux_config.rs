//! `tst_demux_config_t` opaque builder + supporting C-ABI enums.
//!
//! Mirrors the `tst_mux_config_t` shape from plan #14: heap-allocated
//! opaque builder, mutating setters returning `i32` codes, `_free`
//! releases. The receiver clones what it needs at `_open_with_config`
//! time; the caller still owns the builder and must `_free` it.

use libc::c_int;

/// `repr(i32)` mirror of `tst_core::mpegts::demux::StrictMode`.
///
/// Four-valued enum: `Off=0` (default, lenient), `TimingOnly=1`,
/// `DescriptorsOnly=2`, `Full=3`. cbindgen emits parallel `#define
/// TST_STRICT_MODE_*` blocks for C callers.
///
/// NOTE: this differs from the receiver-surface design doc §7.1, which
/// originally specified `0=Off, 1=KlvOnly, 2=All`. The actual Rust enum
/// is 4-valued; this mapping is the truth.
#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TstStrictMode {
    Off = 0,
    TimingOnly = 1,
    DescriptorsOnly = 2,
    Full = 3,
}

impl TstStrictMode {
    #[allow(dead_code)] // used in later Phase 3 tasks
    pub(crate) fn from_c_int(v: c_int) -> Option<Self> {
        match v {
            0 => Some(Self::Off),
            1 => Some(Self::TimingOnly),
            2 => Some(Self::DescriptorsOnly),
            3 => Some(Self::Full),
            _ => None,
        }
    }

    #[allow(dead_code)] // used in later Phase 3 tasks
    pub(crate) fn to_rust(self) -> tst_core::mpegts::demux::StrictMode {
        use tst_core::mpegts::demux::StrictMode;
        match self {
            Self::Off => StrictMode::Off,
            Self::TimingOnly => StrictMode::TimingOnly,
            Self::DescriptorsOnly => StrictMode::DescriptorsOnly,
            Self::Full => StrictMode::Full,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_mode_round_trip() {
        for v in 0..=3 {
            let m = TstStrictMode::from_c_int(v).expect("recognized");
            assert_eq!(m as i32, v);
        }
        assert!(TstStrictMode::from_c_int(-1).is_none());
        assert!(TstStrictMode::from_c_int(4).is_none());
    }
}
