# Changelog

All notable changes to this project are documented in this file.

Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) style.
Versioning: [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## Unreleased — Phase 1 SemVer ratchet (2026-05-08)

### Breaking

- **`MuxError` field-tag retypes:** `AmbiguousTarget.kind` and
  `InvalidStreamHandle.kind` changed from `&'static str` to `StreamKind`;
  `InvalidTeletextField.field` changed from `&'static str` to `TeletextField`.
  Display output is unchanged — both new enums implement `Display` with the
  same human-readable strings as before.

- **Two new `MuxError` variants:** `DescriptorIndexOutOfRange { kind:
  StreamKind, index: u32, program_number: u16 }` and `AbsIndexOutOfRange {
  abs_idx: u32, len: u32, program_number: u16 }`. The five
  `MuxerConfigBuilder::stream_descriptors_for_*` / `ProgramBuilder`
  out-of-range paths previously panicked; they now store a deferred typed
  error and surface it from `MuxerConfigBuilder::build()`. First-error-wins.

- **`#[non_exhaustive]` added to 37 public error enums** across `tst-core`
  (10 enums), `tst-pipeline` (5 enums), and `tst-srt` (12 newly attributed +
  `UrlError` which already had it = 13). Future variants on these enums will
  not be SemVer-breaking, but external `match` arms now require a wildcard
  (`_`) arm. Full list: `MuxError`, `DemuxError`, `AuCellError`,
  `DescriptorError`, `DescriptorParseError`, `PsiParseError`, `TsParseError`,
  `CodecParseError`, `TransportError` (tst-core, 10 total including
  `VTargetPackError` which already had it); `MuxSenderError`, `SenderError`,
  `DemuxReceiverError`, `TsFramingError`, `GapBufferError` (tst-pipeline);
  `PassphraseError`, `StreamIdError`, `PacketFilterError`, `AddrError`,
  `OptionError`, `IoError`, `ConnectError`, `BindError`, `AcceptError`,
  `SendError`, `RecvError`, `Error`, `UrlError` (tst-srt).

- **`Pts90khz` / `Pcr27mhz` inner field is now private.** The public tuple
  field (`Pts90khz(pub i64)`) allowed bypassing the typed-time invariant.
  Use `::new(ticks)` to construct and `.as_ticks()` to read raw ticks.
  Existing call sites using `Pts90khz::from_millis` / `from_pts` /
  arithmetic operators are unaffected.

- **Mux config type rename cascade:** `mpegts::mux::Config` →
  `MuxerConfig`; `ConfigBuilder` → `MuxerConfigBuilder`; `ProgramConfig` →
  `MuxerProgramConfig`; `ProgramBuilder` → `MuxerProgramBuilder`. The old
  names are gone with no aliases.

- **`MuxSender::new` arg order swapped** from `(config, transport)` to
  `(transport, config)`, matching `Sender::new` and `RawSender::new`.

- **`Role` enum renamed variants and default changed:** `Role::DemuxReceiver`
  → `Role::Receiver`; `Role::MuxSender` → `Role::Sender`. The dead
  `Role::Unspecified` alias is removed. `Role` now `Default`s to
  `Role::Receiver` (was `Role::Unspecified`; the libsrt-level socket mode
  behavior is unchanged — `merge_receiver_defaults` / `merge_sender_defaults`
  still select the right SRTO option set). `Role` is now `#[non_exhaustive]`.

- **`ParseError` types disambiguated:** `mpegts::descriptors::ParseError` →
  `DescriptorParseError`; `codec::ParseError` → `CodecParseError`.
  `DescriptorError` (build-side) remains distinct from `DescriptorParseError`
  (parse-side).

- **Cancel-handle return shape unified.** All nine
  `Transport::cancel_handle()` call sites now return
  `Option<Arc<dyn TransportCancel + Send + Sync>>` consistently. Previous
  shape was a mix of `Box<dyn …>`, shared references, and bare
  `Arc<…>` without the `dyn` bound.

- **Stats return shape unified.** `Sender::stats()` and the inner
  `framing.stats()` now return an owned `SenderStats` snapshot. Callers that
  stored a `&SenderStats` reference must switch to owning the value.

- **`tst-core` crate root re-exports are now explicit.** Wildcard glob
  imports of `tst_core::*` previously escaped every future addition to
  `error.rs` and `transport.rs` implicitly. The exports are now a finite,
  documented list; future internal additions no longer appear automatically.

- **New `tst-srt` crate-root re-exports:** `RecvError`, `SendError`,
  `ConnectError`, and `BindError` are now reachable as `tst_srt::RecvError`
  etc. (previously required the full `tst_srt::srt::error::*` path).

- **`UasDatalinkLs` re-exported at `tst_core` crate root** (previously
  `tst_core::klv::st0601::UasDatalinkLs` only).

- **`Demuxer::programs_for_test` scoped to `pub(crate)` + `#[cfg(test)]`.**
  Was `pub`. White-box PAT/PMT diffing tests are now unit tests inside
  `demuxer.rs`'s `mod tests` block instead of integration tests.

- **`VideoStreamHandle::for_test` and `KlvStreamHandle::for_test` deleted.**
  For valid handles use `pack(prog, within)`; for out-of-range sentinel values
  use `from_raw(u32::MAX)`.

- **`klv::pack::Iter` visibility tightened to `pub(crate)`** — deferred to
  Phase 5 (the public-facing iterator surface requires the god-module split
  to settle first; see `docs/plans/2026-05-08-phase-1-semver-ratchet.md`
  Task 1.3.4 for rationale). External consumers iterating KLV packs should
  use the higher-level typed-set decoders (`klv::st0601`, `klv::st0102`,
  `klv::st0903`).

### Internal

- 44 `#[allow(dead_code)]` annotations swept across 15 files. 40 were
  cascade-pattern artifacts (helpers landed before consumers in earlier plans;
  consumers shipped but the annotations were never removed). `cargo clippy -D
  warnings` stays green post-removal. 7 newly-flagged items deleted as
  genuinely dead (including `Handle<T>::into_raw`, `Handle<T>::from_raw`,
  `MemTransport::taken`, and two `_unused()` sentinel functions in
  `klv/pack.rs` and `klv/imapb.rs`); 2 gated correctly under `#[cfg(test)]`.

- Internal codec-parser substrate types (`BitReader` in `codec::h265`,
  `Av1BitReader` in `codec::av1`) marked `#[doc(hidden)]`. Their public
  visibility was a cross-module necessity, not consumer surface. Phase 5's
  god-module split will relocate them.

- Three hand-rolled `Display` + `std::error::Error` implementations migrated
  to `thiserror` derives: `CodecParseError`, `AuCellError`, `DescriptorError`.
  Externally-observable `Display` strings are unchanged (verified by
  regression tests).

### CI

- **`cargo public-api` baselines committed** for `tst-core`, `tst-pipeline`,
  and `tst-srt` (`crates/<name>/public-api.txt`). A new CI step diffs the
  current surface against the baseline and fails on any unintended drift.
  Intentional SemVer-breaking changes must update the baseline file in the
  same commit so reviewers can audit the delta. `tst-c` is excluded: its
  public surface is a C ABI tracked by `tstrans.h` (cbindgen), not a Rust
  API; `cargo-public-api` cannot handle the `lib "tstrans"` / `package
  "tst-c"` name mismatch.

- **`#[non_exhaustive]` count guard added.** CI asserts the count of
  `#[non_exhaustive]` attributes across all crate source files never
  decreases below the Phase 1 baseline of 37. New error enums must bump the
  `BASELINE` constant in the same commit.
