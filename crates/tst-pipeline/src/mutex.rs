//! Internal lock abstraction so the sender shells compile both with the
//! standard library and `#![no_std]`+`alloc`.
//!
//! - Under `std`, [`ShellMutex`] is a type alias for [`std::sync::Mutex`] —
//!   poison semantics and behavior are exactly std's (zero divergence on the
//!   gating platforms).
//! - Under `no_std`, it is a thin newtype over [`spin::Mutex`] — no
//!   poisoning, no priority inheritance, no interrupt masking. The shells'
//!   documented contract (one sender per task) keeps the lock uncontended
//!   in correct programs: it exists to satisfy `Sync`, not to arbitrate
//!   cross-task sharing. Sharing one sender across preemptive tasks anyway
//!   can livelock — a higher-priority task spins forever against a
//!   preempted lock-holder (classic priority inversion). That is a
//!   contract violation, not a supported mode. `lock()` returns
//!   `Result<_, ()>` (always `Ok`) so the call sites — which use
//!   `if let Ok(..)` / `match` / `.map_err(|_| ..)?` and never name
//!   `PoisonError` — compile unchanged against both backends.

#[cfg(feature = "std")]
pub(crate) type ShellMutex<T> = std::sync::Mutex<T>;

// `spin` is referenced only by the no_std backend below, and Cargo cannot
// express "dependency only when a feature is OFF" — so on std builds it looks
// unused to cargo's (nightly) `unused_dependencies` lint. The conventional
// `use … as _` marks it as deliberately present.
#[cfg(feature = "std")]
use spin as _;

#[cfg(not(feature = "std"))]
pub(crate) use no_std_impl::ShellMutex;

#[cfg(not(feature = "std"))]
mod no_std_impl {
    use spin::{Mutex, MutexGuard};

    /// Spin-backed mutex with a `Result`-returning `lock()` mirroring the
    /// shape of `std::sync::Mutex` (minus poisoning, which cannot occur
    /// without unwinding).
    pub(crate) struct ShellMutex<T>(Mutex<T>);

    impl<T> ShellMutex<T> {
        pub(crate) fn new(value: T) -> Self {
            ShellMutex(Mutex::new(value))
        }

        /// Always `Ok` — a spinlock cannot be poisoned. The `Err = ()` arm
        /// exists only so `if let Ok(..)` / `.map_err(..)` call sites that
        /// were written against `std::sync::Mutex` compile verbatim.
        pub(crate) fn lock(&self) -> Result<MutexGuard<'_, T>, ()> {
            Ok(self.0.lock())
        }
    }
}
