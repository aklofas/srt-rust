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

        /// Non-blocking lock attempt. `None` on contention; spin-backed
        /// mutexes cannot poison, so that's the only failure mode.
        pub(crate) fn try_lock(&self) -> Option<MutexGuard<'_, T>> {
            self.0.try_lock()
        }
    }
}

/// Outcome of a non-blocking lock attempt on a [`ShellMutex`], unified
/// across both backends via the [`try_lock`] free function so a call site
/// (`MuxSender::close`) written once compiles identically against `std`
/// and `no_std`. Spin-backed `no_std` mutexes cannot poison — the `no_std`
/// arm of [`try_lock`] never constructs `Poisoned` — but the variant stays
/// so the match at the call site doesn't need per-backend `#[cfg]`.
pub(crate) enum TryLockOutcome<G> {
    /// Lock acquired uncontended.
    Acquired(G),
    /// Another holder has the lock right now.
    WouldBlock,
    /// The lock is poisoned (a previous holder panicked while it was
    /// held); the guard is still handed back for best-effort recovery.
    /// `std` only — the `no_std` backend never constructs this arm since
    /// a spin mutex cannot poison.
    #[cfg_attr(not(feature = "std"), allow(dead_code))]
    Poisoned(G),
}

#[cfg(feature = "std")]
pub(crate) fn try_lock<T>(m: &ShellMutex<T>) -> TryLockOutcome<std::sync::MutexGuard<'_, T>> {
    match m.try_lock() {
        Ok(g) => TryLockOutcome::Acquired(g),
        Err(std::sync::TryLockError::WouldBlock) => TryLockOutcome::WouldBlock,
        Err(std::sync::TryLockError::Poisoned(e)) => TryLockOutcome::Poisoned(e.into_inner()),
    }
}

#[cfg(not(feature = "std"))]
pub(crate) fn try_lock<T>(m: &ShellMutex<T>) -> TryLockOutcome<spin::MutexGuard<'_, T>> {
    match m.try_lock() {
        Some(g) => TryLockOutcome::Acquired(g),
        None => TryLockOutcome::WouldBlock,
    }
}
