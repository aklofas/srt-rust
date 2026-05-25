//! Dyn-erased pipeline shell aliases for FFI bindings.
//!
//! These six aliases re-export the generic pipeline shells with `Box<dyn
//! Transport>` / `Box<dyn RecvTransport>` substituted in. Bindings code
//! (`tst-jni`, `tst-uniffi`, `tst-pyo3`) targets one concrete type per shell
//! instead of cubing per-`T` instantiation.
//!
//! Rust callers with a custom transport keep the generic shape (e.g.
//! `MuxSender<MyTransport>`); the aliases are purely a binding-author
//! convenience.
//!
//! See the [binding-authors guide](https://github.com/aklofas/ts-transformer/blob/main/ts-transformer/docs/reference/binding-authors.md)
//! for worked examples per language.

pub use crate::demux_receiver::BoxedDemuxReceiver;
pub use crate::mux_sender::BoxedMuxSender;
pub use crate::raw_receiver::BoxedRawReceiver;
pub use crate::raw_sender::BoxedRawSender;
pub use crate::receiver::BoxedReceiver;
pub use crate::sender::BoxedSender;
