//! Float-math shim for the `no_std` build.
//!
//! `core` exposes no transcendental/rounding float intrinsics (those live in
//! the platform libm that `std` links). Under `no_std` we route through the
//! `libm` crate; under `std` we forward to the inherent `f64` methods. The
//! [`FloatExt`] trait keeps call sites unchanged (`x.log2()`, `x.floor()`, …).

/// Float operations the KLV IMAPB / ST 0601 mapping code needs that are not
/// available on `f64` in a `no_std` (`core`-only) build.
pub(crate) trait FloatExt {
    fn log2(self) -> Self;
    fn ceil(self) -> Self;
    fn floor(self) -> Self;
    fn round(self) -> Self;
    fn powf(self, n: Self) -> Self;
}

#[cfg(not(feature = "std"))]
impl FloatExt for f64 {
    fn log2(self) -> Self {
        libm::log2(self)
    }
    fn ceil(self) -> Self {
        libm::ceil(self)
    }
    fn floor(self) -> Self {
        libm::floor(self)
    }
    fn round(self) -> Self {
        libm::round(self)
    }
    fn powf(self, n: Self) -> Self {
        libm::pow(self, n)
    }
}
