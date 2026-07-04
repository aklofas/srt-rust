//! Shared utilities for the Python bindings.

use pyo3::intern;
use pyo3::prelude::*;
use pyo3::types::PyBytes;

/// Coerce a Python bytes-like argument (`bytes`, `bytearray`, `memoryview`,
/// NumPy `uint8`) to an owned `Bound<'py, PyBytes>` strong reference.
///
/// Fast path for `bytes`: zero-copy clone of the bound reference.
/// Fallback: calls Python's `bytes()` built-in to coerce the argument,
/// which copies the data once into an immutable `bytes` object.
///
/// The `bytes()` fallback accepts strictly more than the stubs'
/// `_BytesLike` promise: an `int` yields that many zero bytes and any
/// iterable of ints is materialized (`bytes(5)`, `bytes([1, 2, 3])`).
/// That widening is deliberate — every transport sender has coerced this
/// way since the pattern was introduced, and narrowing here would make
/// `Muxer.push_*` reject inputs its wrapping senders accept. The stubs'
/// `_BytesLike` annotation is the static-analysis guard against misuse.
///
/// PyO3 0.22 abi3-py310 cannot extract `&[u8]` from `bytearray` or
/// `memoryview` — only from `bytes`. This helper bridges that gap by
/// accepting any bytes-like and returning a `PyBytes` whose `.as_bytes()`
/// borrow lives for as long as the returned value is on the stack. That
/// makes it safe to pass the resulting `&[u8]` across a subsequent
/// `py.allow_threads()` call.
///
/// Raises `TypeError` if `arg` cannot be passed to `bytes()`.
pub(crate) fn coerce_bytes_like<'py>(
    py: Python<'py>,
    arg: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyBytes>> {
    if let Ok(b) = arg.downcast::<PyBytes>() {
        return Ok(b.clone());
    }
    py.import_bound("builtins")?
        .getattr(intern!(py, "bytes"))?
        .call1((arg,))?
        .downcast_into::<PyBytes>()
        .map_err(|e| e.into())
}

/// Format a `host:port` string, bracketing IPv6 literals so the result
/// parses through `SocketAddr` / `ToSocketAddrs`.
///
/// `host` must be the plain hostname or IP literal (without brackets or
/// port). The function adds `[…]` iff `host` contains a colon and does
/// not already start with `[`.
#[allow(dead_code)] // transport-feature-gated callers; unused in minimal builds
pub(crate) fn join_host_port(host: &str, port: u16) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}
