//! Sender-side opaque builder handles: `TstSenderConfig`,
//! `TstRawSenderConfig`, and `TstReconnectPolicy`.
//!
//! Each builder is a `Box<T>`. `_open` clones the inner before consuming
//! it, so the caller may free immediately after a successful open. The
//! `TstTsFramingMode` and `TstOverflowPolicy` enums that parameterize the
//! setters live alongside the builders since they are referenced nowhere
//! else.

use crate::error::{TstError, set_last_error};
use crate::panic::ffi_catch;
use std::time::Duration;
use tst_pipeline::{
    BackoffStrategy, OverflowPolicy, RawSenderConfig, ReconnectPolicy, SenderConfig, TsFramingMode,
};

// ------------------------------------------------------------------
// tst_sender_config_t
// ------------------------------------------------------------------

pub struct TstSenderConfig {
    pub(crate) inner: SenderConfig,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_sender_config_new() -> *mut TstSenderConfig {
    ffi_catch(std::ptr::null_mut(), || {
        Box::into_raw(Box::new(TstSenderConfig {
            inner: SenderConfig::default(),
        }))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_sender_config_free(p: *mut TstSenderConfig) {
    ffi_catch((), || {
        if !p.is_null() {
            unsafe { drop(Box::from_raw(p)) };
        }
    })
}

#[repr(C)]
#[derive(Clone, Copy)]
pub enum TstTsFramingMode {
    Recover = 0,
    Strict = 1,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_sender_config_set_framing_mode(
    p: *mut TstSenderConfig,
    mode: TstTsFramingMode,
) -> libc::c_int {
    ffi_catch(TstError::Internal as i32, || {
        let Some(cfg) = (unsafe { p.as_mut() }) else {
            set_last_error(TstError::InvalidConfig, "null config pointer");
            return TstError::InvalidConfig as i32;
        };
        cfg.inner.framing_mode = match mode {
            TstTsFramingMode::Recover => TsFramingMode::Recover,
            TstTsFramingMode::Strict => TsFramingMode::Strict,
        };
        0
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_sender_config_set_max_unsynced_bytes(
    p: *mut TstSenderConfig,
    n: usize,
) -> libc::c_int {
    ffi_catch(TstError::Internal as i32, || {
        let Some(cfg) = (unsafe { p.as_mut() }) else {
            set_last_error(TstError::InvalidConfig, "null config pointer");
            return TstError::InvalidConfig as i32;
        };
        cfg.inner.max_unsynced_bytes = n;
        0
    })
}

// ------------------------------------------------------------------
// tst_raw_sender_config_t (empty today; reserved for future setters)
// ------------------------------------------------------------------

pub struct TstRawSenderConfig {
    pub(crate) inner: RawSenderConfig,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_raw_sender_config_new() -> *mut TstRawSenderConfig {
    ffi_catch(std::ptr::null_mut(), || {
        Box::into_raw(Box::new(TstRawSenderConfig {
            inner: RawSenderConfig::default(),
        }))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_raw_sender_config_free(p: *mut TstRawSenderConfig) {
    ffi_catch((), || {
        if !p.is_null() {
            unsafe { drop(Box::from_raw(p)) };
        }
    })
}

// ------------------------------------------------------------------
// tst_reconnect_policy_t
// ------------------------------------------------------------------

pub struct TstReconnectPolicy {
    pub(crate) inner: ReconnectPolicy,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_reconnect_policy_new() -> *mut TstReconnectPolicy {
    ffi_catch(std::ptr::null_mut(), || {
        Box::into_raw(Box::new(TstReconnectPolicy {
            inner: ReconnectPolicy::default(),
        }))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_reconnect_policy_free(p: *mut TstReconnectPolicy) {
    ffi_catch((), || {
        if !p.is_null() {
            unsafe { drop(Box::from_raw(p)) };
        }
    })
}

/// Set max reconnect attempts. `n < 0` means retry forever.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_reconnect_policy_set_max_attempts(
    p: *mut TstReconnectPolicy,
    n: i32,
) -> libc::c_int {
    ffi_catch(TstError::Internal as i32, || {
        let Some(cfg) = (unsafe { p.as_mut() }) else {
            set_last_error(TstError::InvalidConfig, "null config pointer");
            return TstError::InvalidConfig as i32;
        };
        cfg.inner.max_attempts = if n < 0 { None } else { Some(n as u32) };
        0
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_reconnect_policy_set_backoff_constant_ms(
    p: *mut TstReconnectPolicy,
    ms: u32,
) -> libc::c_int {
    ffi_catch(TstError::Internal as i32, || {
        let Some(cfg) = (unsafe { p.as_mut() }) else {
            set_last_error(TstError::InvalidConfig, "null config pointer");
            return TstError::InvalidConfig as i32;
        };
        cfg.inner.backoff = BackoffStrategy::Constant(Duration::from_millis(ms as u64));
        0
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_reconnect_policy_set_backoff_exponential_ms(
    p: *mut TstReconnectPolicy,
    base_ms: u32,
    max_ms: u32,
) -> libc::c_int {
    ffi_catch(TstError::Internal as i32, || {
        let Some(cfg) = (unsafe { p.as_mut() }) else {
            set_last_error(TstError::InvalidConfig, "null config pointer");
            return TstError::InvalidConfig as i32;
        };
        cfg.inner.backoff = BackoffStrategy::Exponential {
            base: Duration::from_millis(base_ms as u64),
            max: Duration::from_millis(max_ms as u64),
        };
        0
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_reconnect_policy_set_gap_buffer_capacity(
    p: *mut TstReconnectPolicy,
    n: usize,
) -> libc::c_int {
    ffi_catch(TstError::Internal as i32, || {
        let Some(cfg) = (unsafe { p.as_mut() }) else {
            set_last_error(TstError::InvalidConfig, "null config pointer");
            return TstError::InvalidConfig as i32;
        };
        cfg.inner.gap_buffer_capacity = n;
        0
    })
}

#[repr(C)]
#[derive(Clone, Copy)]
pub enum TstOverflowPolicy {
    DropOldest = 0,
    Reject = 1,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_reconnect_policy_set_overflow_policy(
    p: *mut TstReconnectPolicy,
    policy: TstOverflowPolicy,
) -> libc::c_int {
    ffi_catch(TstError::Internal as i32, || {
        let Some(cfg) = (unsafe { p.as_mut() }) else {
            set_last_error(TstError::InvalidConfig, "null config pointer");
            return TstError::InvalidConfig as i32;
        };
        cfg.inner.overflow_policy = match policy {
            TstOverflowPolicy::DropOldest => OverflowPolicy::DropOldest,
            TstOverflowPolicy::Reject => OverflowPolicy::Reject,
        };
        0
    })
}
