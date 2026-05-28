//! `tst_rist_*` C ABI entry points. Gated on `feature = "rist"`.
//!
//! Exposes constructors for `RistTransport`/`RistRecvTransport` (RIST
//! Simple + Main profiles, librist v0.2.10) and the encryption-key
//! shim (AES-128/192/256 pre-shared keys via mbedTLS). Encryption knobs
//! are inert when `tst-rist` is built `--no-default-features`
//! (returns `TstError::RistEncryptionDisabled = -41`).
