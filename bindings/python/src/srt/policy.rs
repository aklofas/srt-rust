//! `ReconnectPolicy`, `BackoffStrategy`, `OverflowPolicy`.
//!
//! Frozen PyClasses + IntEnum mirror of `tst_pipeline::reconnect` types.
//! Construct via classmethods (`BackoffStrategy`) or kwargs
//! (`ReconnectPolicy`). Pure-Python ergonomics — no transport
//! interactions, no GIL release boundaries, no error mapping
//! involvement.
//!
//! Inner Rust types are exposed via `pub(crate)` fields so
//! `ManagedSender` etc. can lift them into
//! `ManagedTransport::new(.., factory, policy)` without re-translation.
//!
//! ## Drift from plan sketch
//!
//! The plan sketched `OverflowPolicy::DROP_NEWEST` + `BLOCK`, but the
//! real Rust enum at `tst_pipeline::reconnect::gap_buffer` only has
//! `DropOldest` (default) and `Reject`. This module mirrors what Rust
//! actually ships: `DROP_OLDEST = 0`, `REJECT = 1`.

#![allow(unsafe_op_in_unsafe_fn, clippy::useless_conversion)]

use std::time::Duration;

use pyo3::prelude::*;
use pyo3::types::PyType;

use tst_pipeline::{
    BackoffStrategy as RustBackoff, OverflowPolicy as RustOverflow, ReconnectPolicy as RustPolicy,
};

pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyBackoffStrategy>()?;
    m.add_class::<PyOverflowPolicy>()?;
    m.add_class::<PyReconnectPolicy>()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// PyBackoffStrategy — sum-type frozen PyClass with classmethods.
// ---------------------------------------------------------------------------

/// Backoff strategy for reconnect attempts.
///
/// Construct via the two classmethods: `BackoffStrategy.constant(ms=...)`
/// or `BackoffStrategy.exponential(base_ms=..., max_ms=...)`. The
/// inspect-side accessors (`kind`, `base_ms`, `max_ms`) work uniformly
/// across both variants — for the constant variant, `base_ms == max_ms`
/// equals the fixed wait.
#[pyclass(name = "BackoffStrategy", module = "tstrans.srt", frozen)]
#[derive(Clone)]
pub struct PyBackoffStrategy {
    pub(crate) inner: RustBackoff,
}

#[pymethods]
impl PyBackoffStrategy {
    /// Fixed wait between reconnect attempts.
    #[classmethod]
    pub fn constant(_cls: &Bound<'_, PyType>, ms: u32) -> Self {
        Self {
            inner: RustBackoff::Constant(Duration::from_millis(ms.into())),
        }
    }

    /// Exponential backoff: wait = base * 2^(attempt-1), capped at max.
    ///
    /// Raises `ValueError` if `max_ms < base_ms`.
    #[classmethod]
    #[pyo3(signature = (*, base_ms, max_ms))]
    pub fn exponential(_cls: &Bound<'_, PyType>, base_ms: u32, max_ms: u32) -> PyResult<Self> {
        if max_ms < base_ms {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "max_ms must be >= base_ms",
            ));
        }
        Ok(Self {
            inner: RustBackoff::Exponential {
                base: Duration::from_millis(base_ms.into()),
                max: Duration::from_millis(max_ms.into()),
            },
        })
    }

    /// `"constant"` or `"exponential"`.
    #[getter]
    pub fn kind(&self) -> &'static str {
        match self.inner {
            RustBackoff::Constant(_) => "constant",
            RustBackoff::Exponential { .. } => "exponential",
        }
    }

    /// Base wait in milliseconds. For the constant variant, equals the
    /// fixed wait.
    #[getter]
    pub fn base_ms(&self) -> u64 {
        match self.inner {
            RustBackoff::Constant(d) => duration_to_ms(d),
            RustBackoff::Exponential { base, .. } => duration_to_ms(base),
        }
    }

    /// Maximum wait in milliseconds. For the constant variant, equals
    /// the fixed wait (no growth).
    #[getter]
    pub fn max_ms(&self) -> u64 {
        match self.inner {
            RustBackoff::Constant(d) => duration_to_ms(d),
            RustBackoff::Exponential { max, .. } => duration_to_ms(max),
        }
    }

    fn __repr__(&self) -> String {
        match self.inner {
            RustBackoff::Constant(d) => {
                format!("BackoffStrategy.constant(ms={})", duration_to_ms(d))
            }
            RustBackoff::Exponential { base, max } => format!(
                "BackoffStrategy.exponential(base_ms={}, max_ms={})",
                duration_to_ms(base),
                duration_to_ms(max),
            ),
        }
    }
}

fn duration_to_ms(d: Duration) -> u64 {
    d.as_millis().min(u128::from(u64::MAX)) as u64
}

// ---------------------------------------------------------------------------
// PyOverflowPolicy — IntEnum-shaped frozen PyClass.
// ---------------------------------------------------------------------------

/// What `ManagedTransport` does when the gap buffer is full and a new
/// message arrives during an outage.
///
/// - `DROP_OLDEST` (default): evict the front of the queue to make room.
/// - `REJECT`: refuse to enqueue; surface an error to the caller.
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
#[pyclass(name = "OverflowPolicy", module = "tstrans.srt", eq, eq_int)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PyOverflowPolicy {
    DROP_OLDEST = 0,
    REJECT = 1,
}

impl From<PyOverflowPolicy> for RustOverflow {
    fn from(p: PyOverflowPolicy) -> Self {
        match p {
            PyOverflowPolicy::DROP_OLDEST => RustOverflow::DropOldest,
            PyOverflowPolicy::REJECT => RustOverflow::Reject,
        }
    }
}

impl From<RustOverflow> for PyOverflowPolicy {
    fn from(p: RustOverflow) -> Self {
        match p {
            RustOverflow::DropOldest => PyOverflowPolicy::DROP_OLDEST,
            RustOverflow::Reject => PyOverflowPolicy::REJECT,
        }
    }
}

// ---------------------------------------------------------------------------
// PyReconnectPolicy.
// ---------------------------------------------------------------------------

/// Tuning for `ManagedSender` / `ManagedReceiver` reconnect behavior.
///
/// Defaults mirror `tst_pipeline::ReconnectPolicy::default()`:
/// - `max_attempts = 10`
/// - `backoff = BackoffStrategy.exponential(base_ms=100, max_ms=10_000)`
/// - `gap_buffer_capacity = 256`
/// - `overflow_policy = OverflowPolicy.DROP_OLDEST`
///
/// Raises `ValueError` if `gap_buffer_capacity == 0`.
#[pyclass(name = "ReconnectPolicy", module = "tstrans.srt", frozen)]
#[derive(Clone)]
pub struct PyReconnectPolicy {
    pub(crate) inner: RustPolicy,
}

#[pymethods]
impl PyReconnectPolicy {
    #[new]
    #[pyo3(signature = (
        *,
        max_attempts = Some(10),
        backoff = None,
        gap_buffer_capacity = 256,
        overflow_policy = PyOverflowPolicy::DROP_OLDEST,
    ))]
    pub fn new(
        max_attempts: Option<u32>,
        backoff: Option<PyBackoffStrategy>,
        gap_buffer_capacity: usize,
        overflow_policy: PyOverflowPolicy,
    ) -> PyResult<Self> {
        if gap_buffer_capacity == 0 {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "gap_buffer_capacity must be > 0",
            ));
        }
        let backoff_inner = backoff.map(|b| b.inner).unwrap_or_default();
        Ok(Self {
            inner: RustPolicy {
                max_attempts,
                backoff: backoff_inner,
                gap_buffer_capacity,
                overflow_policy: overflow_policy.into(),
                ..Default::default()
            },
        })
    }

    /// Maximum reconnect attempts before giving up. `None` = retry
    /// forever.
    #[getter]
    pub fn max_attempts(&self) -> Option<u32> {
        self.inner.max_attempts
    }

    /// The configured backoff strategy.
    #[getter]
    pub fn backoff(&self) -> PyBackoffStrategy {
        PyBackoffStrategy {
            inner: self.inner.backoff.clone(),
        }
    }

    /// Gap-buffer capacity in messages.
    #[getter]
    pub fn gap_buffer_capacity(&self) -> usize {
        self.inner.gap_buffer_capacity
    }

    /// What to do when the gap buffer is full during an outage.
    #[getter]
    pub fn overflow_policy(&self) -> PyOverflowPolicy {
        self.inner.overflow_policy.into()
    }

    fn __repr__(&self) -> String {
        let backoff_repr = PyBackoffStrategy {
            inner: self.inner.backoff.clone(),
        }
        .__repr__();
        let overflow_repr = match self.inner.overflow_policy {
            RustOverflow::DropOldest => "OverflowPolicy.DROP_OLDEST",
            RustOverflow::Reject => "OverflowPolicy.REJECT",
        };
        match self.inner.max_attempts {
            Some(n) => format!(
                "ReconnectPolicy(max_attempts={}, backoff={}, gap_buffer_capacity={}, overflow_policy={})",
                n, backoff_repr, self.inner.gap_buffer_capacity, overflow_repr,
            ),
            None => format!(
                "ReconnectPolicy(max_attempts=None, backoff={}, gap_buffer_capacity={}, overflow_policy={})",
                backoff_repr, self.inner.gap_buffer_capacity, overflow_repr,
            ),
        }
    }
}
