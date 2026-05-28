//! Python bindings for tst-udp (`tstrans.udp`). Gated on `feature = "udp"`.
//!
//! Populated by Plan A5b. Bootstrap ships an empty submodule scaffold; the
//! wave adds the transport / publisher PyClasses + builders + error mapping.

use pyo3::prelude::*;

pub(crate) fn register(parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new_bound(parent.py(), "udp")?;
    // Populated by later wave tasks.
    parent.add_submodule(&m)?;
    Ok(())
}
