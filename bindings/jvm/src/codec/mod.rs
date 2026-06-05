//! JVM `org.tstrans.codec` binding module.
//!
//! Task 1 lands the shared value-type marshalling helpers in [`shared`]
//! (enum + `Rational`/`ColorInfo` + `NalUnit`/`Obu` builders). The per-codec
//! parser JNI entry points (`parse_h264_sps`, …) land in the follow-on tasks
//! and reuse these helpers.

pub mod aac;
pub mod av1;
pub mod h264;
pub mod h265;
pub mod h266;
pub mod mpegaudio;
pub mod shared;
