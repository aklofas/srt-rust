//! Smoke test: link against librist and read its version + API version.

use rist_sys::*;

#[test]
fn version_string_is_nonempty() {
    let raw = unsafe { librist_version() };
    assert!(!raw.is_null(), "librist_version() returned NULL");
    let s = unsafe { std::ffi::CStr::from_ptr(raw) }
        .to_str()
        .expect("librist_version() returned non-UTF-8");
    assert!(!s.is_empty(), "librist_version() returned empty string");
    assert!(s.contains("0."), "expected librist 0.x version, got: {s}");
}

#[test]
fn api_version_is_nonempty() {
    let raw = unsafe { librist_api_version() };
    assert!(!raw.is_null(), "librist_api_version() returned NULL");
    let s = unsafe { std::ffi::CStr::from_ptr(raw) }
        .to_str()
        .expect("librist_api_version() returned non-UTF-8");
    assert!(!s.is_empty(), "librist_api_version() returned empty");
}
