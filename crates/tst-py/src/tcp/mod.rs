//! Python bindings for tst-tcp (`tstrans.tcp`). Gated on `feature = "tcp"`.
//!
//! Populated by Plan A5b. Bootstrap ships an empty submodule scaffold; the
//! wave adds the transport / publisher PyClasses + builders + error mapping.

use pyo3::prelude::*;

pub(crate) fn register(parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new_bound(parent.py(), "tcp")?;
    // Populated by later wave tasks.
    parent.add_submodule(&m)?;
    Ok(())
}
