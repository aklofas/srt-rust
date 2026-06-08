//! Leased native-handle registry — the UAF/double-free-free primitive that backs
//! every `org.tstrans.*` native handle.
//!
//! # Why this exists (the bug it kills)
//!
//! The original JNI handle pattern stored a raw `Box::into_raw(...) as jlong` in a
//! Java `long`, and every native method dereferenced it (`&*(h as *const T)` /
//! `&mut *(h as *mut T)`), with `nClose` doing `Box::from_raw`. That has two races:
//!
//! 1. **Use-after-free** — a fresh JNI entry dereferences the pointer to reach the
//!    resource, which can run concurrently with another thread's `close()` freeing
//!    that same pointer.
//! 2. **Double-free** — `close()`'s check-then-`from_raw` is not atomic, so two
//!    `close()` calls can both reach the free.
//!
//! # The fix (decision D1)
//!
//! The Java `long` becomes an **opaque integer key into a process-global registry**,
//! never a pointer. A native method:
//!
//! 1. calls [`HandleRegistry::lease`] — locks the registry, looks the id up in a
//!    **permanent table**, clones the `Arc<Entry<T>>`, unlocks, and operates on the
//!    clone. The lookup cannot UAF because the table outlives every entry and the
//!    lookup happens under the registry lock.
//! 2. on `None` (absent / closed) throws `IllegalStateException` (the caller does
//!    this; the registry just returns `None`).
//!
//! [`HandleRegistry::close`] is atomic + idempotent: it locks the registry, removes
//! the entry (dropping the registry's strong ref), CASes `closed`, fires the entry's
//! `cancel` hook to wake any parked op, and returns the *taken* resource so the
//! per-type `nClose` can run its own teardown (e.g. `rx.close()`) and drop it. Any
//! in-flight lease still holds its own `Arc` clone, so the resource's `Drop` runs
//! only after the last lease releases. A second `close` finds the id gone → no-op.
//!
//! # Design decisions
//!
//! - **`0` sentinel.** Ids are handed out by a monotonic `u64` counter starting at
//!   `1`, so `0` is never a valid key. The Java side already treats `0` as null and
//!   uses `AtomicLong.getAndSet(0)` on `close()` so two threads can't submit the
//!   same live id.
//! - **ABA / key reuse — sidestepped by construction.** The backing store is a
//!   `HashMap<u64, _>` keyed by the *monotonic* counter, NOT a slab with slot reuse.
//!   A `u64` counter never repeats in any realistic process lifetime (~10^19 handle
//!   allocations), so a stale Java handle whose entry was closed will always miss
//!   the map → `lease` returns `None`. There is no slot to reuse and therefore no
//!   ABA window where a stale handle leases a *different* later resource. (The spec
//!   said "Slab"; a monotonic-key `HashMap` satisfies the same invariant — opaque
//!   key, looked up under the registry lock against a permanent table — and avoids
//!   the offset-by-one and ABA bookkeeping a slab would need. Contention is a
//!   non-issue here: handle create/close are rare relative to leases, and a lease is
//!   a single hash lookup + `Arc` clone under a short-held lock.)
//! - **Per-type registries.** [`HandleRegistry<T>`] is generic; each module
//!   instantiates its own `static REGISTRY: LazyLock<HandleRegistry<ThatType>>`. Ids
//!   are therefore per-type — a key minted by the srt-Sender registry is meaningless
//!   to the rtp-DemuxReceiver registry, which is exactly the type-safety we want and
//!   matches how the Java classes already keep separate `handle` fields. A single
//!   type-erased registry would need `Box<dyn Any>` downcasts on every lease for no
//!   benefit, so per-type is the deliberate choice.
//! - **Flexible teardown.** The cancel hook is an `Option<Box<dyn Fn() + Send +
//!   Sync>>` stored on the entry, so a type WITH a cross-thread cancel (e.g. the rtp
//!   `DemuxReceiver`, whose `close()` must wake a parked `recv_event` before taking
//!   the lock) supplies one, and a type WITHOUT a cancel (e.g. a plain `Sender`)
//!   passes `None`. `close` fires the hook, then hands the taken resource back so the
//!   caller runs whatever type-specific teardown it needs. This covers every shape in
//!   the inventory below without hardcoding a cancel type.
//!
//! # Inventory for the A2 migration
//!
//! Every `Jni*` handle type that must move onto this primitive, with its current
//! backing and its teardown shape (so A2 knows what `cancel` hook + post-`close`
//! teardown each one needs):
//!
//! | Module / Java class | Current backing | Cancel hook | Post-close teardown |
//! |---|---|---|---|
//! | `srt::lowlevel` `Socket` | `Box<Socket>` | none | consumed by `intoSender`/`intoReceiver`, or dropped |
//! | `srt::lowlevel` `Listener` (`JniListener`) | `Box<JniListener>` | accept-cancel (close races a parked `accept`) | `drop` (the `Box::from_raw` aborts a parked accept today) |
//! | `srt::transport` `Sender` | `Box<PlSender<SrtTransport>>` | none | `inner.close()` then drop |
//! | `srt::transport` `Receiver` | `Box<PlReceiver<SrtTransport>>` | none | `inner.close()` then drop |
//! | `srt::mux_sender` `MuxSender` | `Box<MuxSender<SrtTransport>>` (`Inner`) | none | drop |
//! | `srt::demux_receiver` `DemuxReceiver` (`JniDemuxReceiver`) | `Box<JniDemuxReceiver>` | none (srt model: public cancel handle + connection-close end iteration) | `inner.close()` then drop |
//! | `srt::mod` `CancelHandle` (`JniCancel`) | `Box<JniCancel>` | n/a (IS a cancel target) | drop |
//! | `srt::managed_basic` `ManagedSender` (`ManagedSenderInner`) | `Box<ManagedSenderInner>` | none | `inner.close()` then drop |
//! | `srt::managed_basic` `ManagedReceiver` (`JniManagedReceiver`) | `Box<JniManagedReceiver>` | none | `inner.close()` then drop |
//! | `srt::managed_convenience` `ManagedMuxSender` (`JniManagedMuxSender`) | `Box<JniManagedMuxSender>` | none | `inner.close()` then drop |
//! | `srt::managed_convenience` `ManagedDemuxReceiver` (`JniManagedDemuxReceiver`) | `Box<JniManagedDemuxReceiver>` | none | `inner.close()` then drop |
//! | `rtp::transport` `Sender` (`JniRtpSender`) | `Box<JniRtpSender>` | `cancel` (wakes a parked `send`) | `inner.close()` then drop |
//! | `rtp::transport` `Receiver` (`JniRtpReceiver`) | `Box<JniRtpReceiver>` | `cancel` (wakes a parked `recv`) | `inner.close()` then drop |
//! | `rtp::mux_sender` `MuxSender` (`Inner`) | `Box<Inner>` | none | drop |
//! | `rtp::demux_receiver` `DemuxReceiver` (`JniRtpDemuxReceiver`) | `Box<JniRtpDemuxReceiver>` | `cancel` (wakes a parked `recv_event` — the headline rtp divergence) | `inner.close()` then drop |
//! | `rtp::client` `RtspSession` (`JniRtspSession`) | `Box<JniRtspSession>` | none (teardown is the session's own `torn_down` flag) | session-specific teardown then drop |
//! | `rtp::client` `RtspCancelHandle` (`JniRtspCancel`) | `Box<JniRtspCancel>` | n/a (IS a cancel target) | drop |
//! | `rtp::server` `RtspServer` (`ServerInner`) | `Box<ServerInner>` | server stop | `stop()` then drop |
//! | `rtp::server` `MountHandle` (`MountInner`) | `Box<MountInner>` | none | drop (mount persists in server until `stop()`) |
//! | `rtp::server` `RtspServerCancelHandle` (`JniRtspServerCancel`) | `Box<JniRtspServerCancel>` | n/a (IS a cancel target) | drop |
//! | `mpegts::muxer` `Muxer` | `Box<Muxer>` | none | drop |
//! | `mpegts::mod` `Demuxer` | `Box<Demuxer>` | none | drop |
//! | `pipeline` `PairingDemuxer` (`org.tstrans.pipeline.Pairer`) | `Box<PairingDemuxer>` | none | drop |
//!
//! Note: the cancel-handle classes (`JniCancel`, `JniRtpCancel`, `JniRtspCancel`,
//! `JniRtspServerCancel`) are themselves cancel *targets* — they hold an
//! `Arc<dyn TransportCancel>` and a flag, not a resource that needs waking. They
//! still benefit from the registry (it kills their own UAF/double-free on `close`),
//! and they simply register with `cancel = None`.

// A1 ships this primitive ahead of its first caller; Task A2 migrates the ~20
// handle types above onto it, at which point these items become live. The tests in
// this module exercise every item, but dead-code analysis runs on the non-test
// build, so allow it crate-locally until A2 lands. Remove once A2 wires callers.
#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// An optional cross-thread cancel hook fired by [`HandleRegistry::close`] *before*
/// the resource is taken, so a parked native op (a blocked `recv`/`accept`) wakes
/// and releases its lease. Types with no parked op pass `None`.
pub(crate) type CancelHook = Box<dyn Fn() + Send + Sync>;

/// A registry entry. Permanent as far as the registry's table is concerned (the
/// table never moves or frees an `Entry` out from under a lease — `close` only
/// removes the registry's own strong `Arc`, and in-flight leases keep theirs).
pub(crate) struct Entry<T> {
    /// The owned native resource, behind a `Mutex` so a long-running op (e.g. a
    /// parked `recv`) holds it while `close` waits to `take()` it. `None` once
    /// `close` has taken it.
    resource: Mutex<Option<T>>,
    /// Fired once by `close` to wake a parked op before taking the lock. `None` for
    /// types with no parked op.
    cancel: Option<CancelHook>,
}

impl<T> Entry<T> {
    /// Lock and operate on the resource. Returns `None` if the resource has already
    /// been taken by `close` (an in-flight lease that lost the race to `close`).
    /// The closure runs under the entry's resource lock.
    ///
    /// `with` is the workhorse a leased native method calls: lease → `with` → done.
    /// Holding the lock across the closure is what lets `close` block until a parked
    /// op releases.
    ///
    /// Serialization: two threads calling a leased method on the *same* handle
    /// serialize on the resource `Mutex` — the second blocks until the first's
    /// closure completes. This is intended (the binding's single-iterator contract),
    /// but a caller should expect the blocking when two ops target one handle.
    pub(crate) fn with<R>(&self, f: impl FnOnce(&mut T) -> R) -> Option<R> {
        let mut guard = self.resource.lock().unwrap_or_else(|e| e.into_inner());
        guard.as_mut().map(f)
    }

    /// `try_lock` variant — returns `Some(None)` if the lock is held (by a parked op),
    /// `Some(Some(r))` if the closure ran, `None` if the resource was already taken.
    /// Used by `isAlive`-style probes that must not block on a parked op.
    pub(crate) fn try_with<R>(&self, f: impl FnOnce(&mut T) -> R) -> TryWith<R> {
        match self.resource.try_lock() {
            Ok(mut guard) => match guard.as_mut() {
                Some(t) => TryWith::Ran(f(t)),
                None => TryWith::Taken,
            },
            Err(std::sync::TryLockError::WouldBlock) => TryWith::Locked,
            Err(std::sync::TryLockError::Poisoned(e)) => {
                let mut guard = e.into_inner();
                match guard.as_mut() {
                    Some(t) => TryWith::Ran(f(t)),
                    None => TryWith::Taken,
                }
            }
        }
    }
}

/// Result of [`Entry::try_with`].
pub(crate) enum TryWith<R> {
    /// The closure ran; here is its value.
    Ran(R),
    /// The resource lock was held by another op (e.g. a parked recv).
    Locked,
    /// The resource was already taken by `close`.
    Taken,
}

/// A process-global, per-type leased-handle registry.
///
/// Construct one `static REGISTRY: LazyLock<HandleRegistry<T>> =
/// LazyLock::new(HandleRegistry::new)` per handle type. The Java `long` handle is
/// the opaque key returned by [`insert`](HandleRegistry::insert).
pub(crate) struct HandleRegistry<T> {
    inner: Mutex<RegistryInner<T>>,
}

struct RegistryInner<T> {
    /// Monotonic, never-reused, never-zero id source. Starts at 1.
    next_id: u64,
    /// The permanent table. Keyed by the monotonic id, so a freed id never reappears
    /// and there is no ABA window.
    table: HashMap<u64, Arc<Entry<T>>>,
}

impl<T> HandleRegistry<T> {
    /// Create an empty registry. Cheap; intended to be passed as the init fn to
    /// `LazyLock::new(HandleRegistry::new)` (evaluated lazily on first use, so it
    /// need not be `const`).
    pub(crate) fn new() -> Self {
        HandleRegistry {
            inner: Mutex::new(RegistryInner {
                next_id: 1,
                table: HashMap::new(),
            }),
        }
    }

    /// Register a resource (no cancel hook) and return its opaque Java handle key.
    /// The returned id is always non-zero.
    pub(crate) fn insert(&self, resource: T) -> u64 {
        self.insert_with_cancel(resource, None)
    }

    /// Register a resource with an optional cross-thread cancel hook. The hook fires
    /// once, inside [`close`](HandleRegistry::close), before the resource is taken —
    /// use it to wake a parked op (e.g. `move || transport_cancel.cancel()`). Returns
    /// the opaque non-zero handle key.
    ///
    /// Single-iterator caveat: the cancel hook fires *exactly once* (when `close`
    /// runs). A lease that begins a *new* parked op *after* `close` has already fired
    /// the hook will NOT be woken by it — the registry guarantees no use-after-free,
    /// not the wake-up of an op parked after cancel. Callers relying on cross-thread
    /// cancel (the rtp `DemuxReceiver` / transport recv/send paths) must therefore
    /// uphold a single-iterator contract: at most one op parked on a handle at a time.
    pub(crate) fn insert_with_cancel(&self, resource: T, cancel: Option<CancelHook>) -> u64 {
        let entry = Arc::new(Entry {
            resource: Mutex::new(Some(resource)),
            cancel,
        });
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let id = inner.next_id;
        // u64 monotonic: in the impossible event of wraparound, skip 0.
        inner.next_id = inner.next_id.wrapping_add(1);
        if inner.next_id == 0 {
            inner.next_id = 1;
        }
        inner.table.insert(id, entry);
        id
    }

    /// Lease the entry for `id`: look it up under the registry lock against the
    /// permanent table and clone its `Arc`. Returns `None` for `0`, an absent id, or
    /// an id already closed (removed from the table). The clone keeps the resource
    /// alive for the duration of the caller's operation even if a concurrent `close`
    /// removes the table's strong ref — so this can never hand out a dangling ref.
    ///
    /// Every leased native method calls this first; `None` → throw
    /// `IllegalStateException` on the Java boundary.
    pub(crate) fn lease(&self, id: u64) -> Option<Arc<Entry<T>>> {
        if id == 0 {
            return None;
        }
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.table.get(&id).cloned()
    }

    /// Lease `id` and run `f` on the resource under its lock — the common A2 path,
    /// folded so call sites get a single `Option<R>` instead of a nested
    /// `Option<Option<R>>`. `None` if `id` is `0`/absent/closed OR the resource was
    /// already taken by a concurrent `close`. A2 maps `None` → throw
    /// `IllegalStateException`.
    pub(crate) fn with<R>(&self, id: u64, f: impl FnOnce(&mut T) -> R) -> Option<R> {
        self.lease(id).and_then(|e| e.with(f))
    }

    /// Lease `id` and run `f` without blocking on a parked op — the `isAlive`-style
    /// probe path. A `0`/absent/closed handle maps to [`TryWith::Taken`] (the entry
    /// is gone, semantically the same as a taken resource for a probe), so call sites
    /// match on the [`TryWith`] enum alone. Otherwise delegates to
    /// [`Entry::try_with`] (`Ran`/`Locked`/`Taken`).
    pub(crate) fn try_with<R>(&self, id: u64, f: impl FnOnce(&mut T) -> R) -> TryWith<R> {
        match self.lease(id) {
            Some(e) => e.try_with(f),
            None => TryWith::Taken,
        }
    }

    /// Close the entry for `id`: remove it from the table (dropping the registry's
    /// strong ref), fire its cancel hook to wake any parked op, then `take()` and
    /// return the resource so the caller can run type-specific teardown + drop it.
    ///
    /// Atomic + idempotent: `table.remove(&id)` under the registry lock is the SOLE
    /// gate — exactly one caller ever extracts the entry, so a second `close` finds
    /// the id gone and returns `None` (and the cancel hook + resource `take` each run
    /// at most once). `0` → `None`.
    ///
    /// The returned `Option<T>` is `Some` only for the winning `close`; the caller
    /// runs e.g. `if let Some(mut r) = REGISTRY.close(id) { r.close(); }`. The
    /// resource's own `Drop` runs when the caller drops the returned value AND every
    /// in-flight lease has released its `Arc` clone (whichever is last).
    pub(crate) fn close(&self, id: u64) -> Option<T> {
        if id == 0 {
            return None;
        }
        // Remove under the registry lock — this is the single atomic gate that makes
        // double-close a no-op: only one caller gets the entry out of the table, so
        // the cancel hook below and the resource `take` each run at most once.
        let entry = {
            let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            inner.table.remove(&id)
        }?;

        // Wake a parked op BEFORE we try to take the resource lock, so a blocked
        // recv/accept releases and we don't deadlock waiting for `take()`.
        if let Some(cancel) = &entry.cancel {
            cancel();
        }

        // Take the resource. A parked op holding the lock releases it once cancel
        // wakes it; we then take ownership and hand it back for teardown.
        let mut guard = entry.resource.lock().unwrap_or_else(|e| e.into_inner());
        guard.take()
    }

    /// Whether `id` is currently registered (live, not closed). Convenience for an
    /// `isAlive`/`isClosed`-style probe that doesn't need to lease. `0` → `false`.
    #[cfg(test)]
    pub(crate) fn contains(&self, id: u64) -> bool {
        if id == 0 {
            return false;
        }
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.table.contains_key(&id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
    use std::thread;

    /// A resource that bumps a shared counter on `Drop`, so tests can assert the
    /// resource's `Drop` runs exactly once and only after the last lease releases.
    struct DropCounter {
        drops: Arc<AtomicUsize>,
        value: u64,
    }

    impl Drop for DropCounter {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn ids_are_nonzero_and_monotonic() {
        let reg: HandleRegistry<u64> = HandleRegistry::new();
        let a = reg.insert(10);
        let b = reg.insert(20);
        let c = reg.insert(30);
        assert_ne!(a, 0);
        assert_ne!(b, 0);
        assert_ne!(c, 0);
        assert!(a < b && b < c, "ids must be monotonic: {a} {b} {c}");
    }

    #[test]
    fn lease_zero_and_unknown_return_none() {
        let reg: HandleRegistry<u64> = HandleRegistry::new();
        assert!(reg.lease(0).is_none());
        assert!(reg.lease(999).is_none());
    }

    #[test]
    fn registry_with_folds_both_layers() {
        let reg: HandleRegistry<u64> = HandleRegistry::new();
        let id = reg.insert(11);
        // Live: folds to Some(R).
        assert_eq!(reg.with(id, |v| *v * 2), Some(22));
        // Absent / closed / zero: all fold to None.
        assert_eq!(reg.with(0, |v| *v), None);
        assert_eq!(reg.with(999, |v| *v), None);
        reg.close(id);
        assert_eq!(reg.with(id, |v| *v), None, "closed id folds to None");
    }

    #[test]
    fn registry_try_with_maps_absent_to_taken() {
        let reg: HandleRegistry<u64> = HandleRegistry::new();
        let id = reg.insert(3);
        assert!(matches!(reg.try_with(id, |v| *v), TryWith::Ran(3)));
        // A 0/absent/closed handle is reported as Taken (entry gone).
        assert!(matches!(reg.try_with(0, |v| *v), TryWith::Taken));
        assert!(matches!(reg.try_with(999, |v| *v), TryWith::Taken));
        reg.close(id);
        assert!(matches!(reg.try_with(id, |v| *v), TryWith::Taken));
    }

    #[test]
    fn lease_after_close_returns_none() {
        let reg: HandleRegistry<u64> = HandleRegistry::new();
        let id = reg.insert(42);
        assert!(reg.lease(id).is_some());
        let taken = reg.close(id);
        assert_eq!(taken, Some(42));
        // The id is gone from the permanent table → no later resource can reuse it
        // (monotonic counter), so lease returns None forever (no ABA).
        assert!(reg.lease(id).is_none());
    }

    #[test]
    fn double_close_is_safe_and_noop() {
        let drops = Arc::new(AtomicUsize::new(0));
        let reg: HandleRegistry<DropCounter> = HandleRegistry::new();
        let id = reg.insert(DropCounter {
            drops: drops.clone(),
            value: 7,
        });

        let first = reg.close(id);
        assert!(first.is_some(), "first close wins");
        assert_eq!(first.unwrap().value, 7);
        assert_eq!(drops.load(Ordering::SeqCst), 1, "resource dropped once");

        // Second close: id already removed → None, no extra drop, no panic.
        let second = reg.close(id);
        assert!(second.is_none(), "second close is a no-op");
        assert_eq!(drops.load(Ordering::SeqCst), 1, "still only one drop");
    }

    #[test]
    fn inflight_lease_keeps_resource_alive_past_close() {
        let drops = Arc::new(AtomicUsize::new(0));
        let reg: HandleRegistry<DropCounter> = HandleRegistry::new();
        let id = reg.insert(DropCounter {
            drops: drops.clone(),
            value: 1,
        });

        // Hold a lease (an in-flight native call).
        let leased = reg.lease(id).expect("leased");

        // Close: takes the resource OUT of the entry and returns it. The Entry Arc is
        // still alive via `leased`, but the DropCounter is now owned by `taken`.
        let taken = reg.close(id);
        assert!(taken.is_some());
        // Drop the taken resource explicitly → that's its one Drop.
        drop(taken);
        assert_eq!(drops.load(Ordering::SeqCst), 1);

        // The lease is still valid as an Arc<Entry>, but the resource is gone:
        // `with` returns None (resource taken), never a dangling ref.
        let r = leased.with(|c| c.value);
        assert!(r.is_none(), "resource already taken → with() yields None");
        drop(leased); // dropping the last Entry Arc frees the (now-empty) Entry
    }

    #[test]
    fn cancel_hook_fires_once_on_close() {
        let fired = Arc::new(AtomicUsize::new(0));
        let reg: HandleRegistry<u64> = HandleRegistry::new();
        let f = fired.clone();
        let id = reg.insert_with_cancel(
            99,
            Some(Box::new(move || {
                f.fetch_add(1, Ordering::SeqCst);
            })),
        );
        reg.close(id);
        reg.close(id); // no-op
        assert_eq!(
            fired.load(Ordering::SeqCst),
            1,
            "cancel hook fires exactly once"
        );
    }

    #[test]
    fn try_with_reports_locked_taken_ran() {
        let reg: HandleRegistry<u64> = HandleRegistry::new();
        let id = reg.insert(5);
        let leased = reg.lease(id).unwrap();
        assert!(matches!(leased.try_with(|v| *v), TryWith::Ran(5)));

        // Simulate a parked op holding the resource lock, then probe with try_with.
        let entry = reg.lease(id).unwrap();
        let entry2 = entry.clone();
        let held = Arc::new(std::sync::Barrier::new(2));
        let release = Arc::new(std::sync::Barrier::new(2));
        let h = held.clone();
        let rel = release.clone();
        let parked = thread::spawn(move || {
            entry2
                .with(|_v| {
                    h.wait(); // signal: lock held
                    rel.wait(); // wait: test done probing
                })
                .unwrap();
        });
        held.wait(); // parked op now holds the lock
        assert!(matches!(entry.try_with(|v| *v), TryWith::Locked));
        release.wait();
        parked.join().unwrap();

        // After close, try_with reports Taken.
        reg.close(id);
        assert!(matches!(entry.try_with(|v| *v), TryWith::Taken));
    }

    /// The headline guarantee: concurrent `lease` while `close` runs never hands out
    /// a dangling ref, double-close never double-frees, and lease-after-close is
    /// always `None`.
    ///
    /// Design: a fixed pool of long-lived worker threads (so thread-spawn cost is
    /// paid once, not per round) all hammer a single shared registry for `ROUNDS`
    /// iterations. Each round the controller inserts a fresh resource and stamps its
    /// id into a shared `AtomicU64`; the leasers spin-lease that id and read its value
    /// (which must equal the round's stamp — a torn/dangling read would mismatch or
    /// crash), while a dedicated closer thread races them to `close` it. After the
    /// round the controller drains with a final `close` and asserts the id is gone.
    /// At the end every inserted resource must have dropped exactly once (no
    /// double-free via the racing closer, no leak from a lost close).
    #[test]
    fn stress_concurrent_lease_and_close() {
        const ROUNDS: u64 = 50_000;
        const LEASERS: usize = 6;

        let reg: Arc<HandleRegistry<DropCounter>> = Arc::new(HandleRegistry::new());
        let total_drops = Arc::new(AtomicUsize::new(0));
        // The id under contention this round (0 between rounds, ignored by lease).
        let current_id = Arc::new(AtomicU64::new(0));
        // The expected `value` stamp for the current id (so a leaser can detect a
        // torn read). Updated before `current_id` is published.
        let current_value = Arc::new(AtomicU64::new(0));
        // Set once at the end so leaser threads exit their hot loop.
        let done = Arc::new(AtomicBool::new(false));

        let mut workers = Vec::with_capacity(LEASERS);
        for _ in 0..LEASERS {
            let reg = reg.clone();
            let current_id = current_id.clone();
            let current_value = current_value.clone();
            let done = done.clone();
            workers.push(thread::spawn(move || {
                while !done.load(Ordering::Acquire) {
                    let id = current_id.load(Ordering::Acquire);
                    if id == 0 {
                        std::hint::spin_loop();
                        continue;
                    }
                    let expected = current_value.load(Ordering::Acquire);
                    // Lease + operate. Either the resource is still live (we read its
                    // value — must equal the stamp, proving no dangling/torn read) or
                    // it's been taken by the closer (with() → None). Both are safe.
                    if let Some(entry) = reg.lease(id) {
                        if let Some(v) = entry.with(|c| c.value) {
                            // A dangling/UAF read would yield a value != the stamp (or
                            // segfault). This assert is the race detector.
                            assert_eq!(v, expected, "leased a live but wrong/torn resource");
                        }
                    }
                }
            }));
        }

        // A dedicated closer thread that races the leasers, plus the controller's own
        // final close, give two threads calling `close(id)` on the same id → exercises
        // double-close idempotency under contention.
        let closer = {
            let reg = reg.clone();
            let current_id = current_id.clone();
            let done = done.clone();
            thread::spawn(move || {
                while !done.load(Ordering::Acquire) {
                    let id = current_id.load(Ordering::Acquire);
                    if id != 0 {
                        if let Some(r) = reg.close(id) {
                            let _ = r.value; // type-specific teardown would go here
                            drop(r);
                        }
                    }
                    std::hint::spin_loop();
                }
            })
        };

        for round in 0..ROUNDS {
            let value = round.wrapping_add(1); // never 0
            current_value.store(value, Ordering::Release);
            let id = reg.insert(DropCounter {
                drops: total_drops.clone(),
                value,
            });
            // Publish the id; leasers + closer now race on it.
            current_id.store(id, Ordering::Release);

            // Give the racing threads a window, then stop targeting this id and ensure
            // it's closed (a no-op if the closer already won). Spin a few times so the
            // race actually happens rather than the controller always winning.
            for _ in 0..16 {
                std::hint::spin_loop();
            }
            current_id.store(0, Ordering::Release);
            if let Some(r) = reg.close(id) {
                drop(r);
            }
            // After this round's close, the id is gone from the permanent table.
            assert!(reg.lease(id).is_none(), "lease after close must be None");
        }

        done.store(true, Ordering::Release);
        for w in workers {
            w.join().unwrap();
        }
        closer.join().unwrap();

        // Exactly one DropCounter per round, dropped exactly once → no double-free,
        // no leak.
        assert_eq!(
            total_drops.load(Ordering::SeqCst),
            ROUNDS as usize,
            "each round's resource must drop exactly once"
        );
    }
}
