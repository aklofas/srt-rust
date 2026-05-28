//! Python bindings for tst-rist (`tstrans.rist`). Gated on `feature = "rist"`.
//!
//! Populated by Plan A5b. Bootstrap ships an empty submodule scaffold; the
//! wave adds the transport / publisher PyClasses + builders + error mapping.

use pyo3::prelude::*;

pub(crate) fn register(parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new_bound(parent.py(), "rist")?;
    // Populated by later wave tasks.
    parent.add_submodule(&m)?;
    Ok(())
}
