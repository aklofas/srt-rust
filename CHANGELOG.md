# Changelog

All notable changes to this project are documented in this file.

Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) style.
Versioning: [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## Unreleased

Phase 1 (SemVer ratchet) and Phase 2 (DX + observability) of the Rust
quality + DX + FFI refactor. Both ship together in the next release.
Plan #39 (examples reorganization) also rides this release.

---

### Phase 3 — FFI-readiness: stream handle opacity (sub-phase 3.3, 2026-05-09)

#### Changed (Phase 3 / handle opacity)

- **`VideoStreamHandle::{pack, unpack, raw, from_raw}` and
  `KlvStreamHandle::{pack, unpack, raw, from_raw}` are now
  `#[doc(hidden)]`.** They remain `pub` (load-bearing for the `tst-c` C
  ABI, which converts handles to/from `uint32_t` across crate lines), but
  they no longer appear in rustdoc. Binding generators (UniFFI / JNI /
  PyO3) that scan the public surface won't surface them; the Java `int` /
  Swift `UInt32` paths to construct invalid handles are eliminated.
  Full `pub(crate)` demotion of these two handle types is deferred to a
  future plan that reshapes `tst-c` to use opaque handles internally.
  Direct Rust callers should obtain handles via `Muxer::add_video_stream`,
  `Muxer::video_handles`, `Muxer::add_klv_stream`, or
  `Muxer::klv_handles` — those are the stable API entry points.

- **`AudioStreamHandle::{pack, unpack}` and
  `SubtitleStreamHandle::{pack, unpack}` are now `pub(crate)`.** No
  external consumers exist (tst-c does not bind audio or subtitle handles
  at the C ABI boundary yet). The `from_raw` / `raw` helpers on these
  types were test-only; they are now `#[cfg(test)] pub(crate)`.

---

### Examples reorganization (2026-05-09)

#### Changed (examples)

- **Examples now live in a workspace-level `tst-examples` crate** at
  `examples/`, organized into 8 task-oriented subfolders
  (`getting-started/`, `sending/`, `muxing/`, `receiving/`,
  `klv-metadata/`, `pairing/`, `codec-parsing/`, `operations/`). The
  per-crate `examples/` directories under `crates/tst-srt/`,
  `crates/tst-pipeline/`, and `crates/tst-core/` are gone.

- **Invocation lines change.** Run any example with
  `cargo run -p tst-examples --example <name>`. The previous forms —
  `cargo run -p tst-srt --example <name>`,
  `cargo run -p tst-pipeline --example <name>`,
  `cargo run -p tst-core --example <name>`, and bare
  `cargo run --example <name>` — no longer resolve. README, cookbook,
  guide-*.md, getting-started.md, architecture.md, and troubleshooting.md
  are all updated; downstream consumers with their own scripts need to
  update theirs.

- **Fixture generators are now `[[bin]]` targets in `tst-core`, not
  examples.** `gen_synthetic_fixtures`, `gen_subtitle_fixtures`,
  `gen_h266_fixtures`, and `gen_av1_fixtures` moved from
  `crates/tst-srt/examples/` to `crates/tst-core/tests/tools/`.
  Invocation: `cargo run -p tst-core --bin <name>`. They're maintainer
  tooling, not learner code; relocating them clarifies the boundary.

- **C examples mirror the same taxonomy** under
  `crates/tst-c/examples/c/{getting-started,muxing}/`. Build commands
  in each file's header updated to the new paths.

#### Added (examples)

- **`getting-started/hello_world.rs`** (Rust) and
  `crates/tst-c/examples/c/getting-started/hello_world.c` (C) — the
  smallest possible mux + KLV round-trip showing what this library
  does. Both produce byte-identical output (752 bytes / 4 packets).
  Designed as the first example a new contributor runs.

- **9 READMEs** — top-level `examples/README.md`, 8 per-category
  READMEs, and `crates/tst-c/examples/c/README.md`. Numbered curriculum
  per category with cookbook backlinks; "diffs from previous"
  call-outs on the cumulative h264 → h265 → h266 → av1 muxing
  progression.

---

### Phase 2 — DX + observability (2026-05-09)

#### Added (Phase 2)

- **CI rail: broken intra-doc links block PRs.** New
  `cargo doc --workspace --no-deps --all-features` step with
  `RUSTDOCFLAGS="-D warnings"`. Warnings of all four classes
  (`broken_intra_doc_links`, `private_intra_doc_links`,
  `invalid_html_tags`, `redundant_explicit_links`) fail the build.

- **CI rail: `cargo test --doc --workspace`.** Doctests now run on
  every PR alongside unit/integration tests.

- **Doctests on 15 top-level public APIs.** `lib.rs` quick-starts on
  `tst-core` + `tst-pipeline` + `tst-srt` (3); sender shells
  `MuxSender` / `Sender` / `RawSender` (3); receiver shells
  `DemuxReceiver` / `Receiver` / `RawReceiver` (3); top-level builders
  `SocketBuilder` / `MuxerConfigBuilder` / `MuxerProgramBuilder` (3);
  KLV typed decoders `klv::st0601::decode` / `klv::st0102::decode` and
  `Pairer::nearest_pts` (3).

- **`# Errors` rustdoc sections on 30 fallible public APIs.** Each
  block names concrete typed error variants and links them via
  intra-doc syntax. Coverage spans 7 `MuxSender::send_*` siblings,
  `Sender::flush`, 8 `Muxer::push_*` methods, 3 `Passphrase`
  constructors, 5 `klv::st0601` entry points, 2 `klv::st0102`
  encoders, and 4 `klv::st0903` entry points.

- **`# Panics` rustdoc sections on 9 caller-observable panic
  surfaces.** Three categories: stream-handle `pack()` debug-asserts
  on out-of-range indices (Video / Klv / Audio / Subtitle, 4 sites);
  internal Mutex poison on `MuxSender` / `ManagedTransport` /
  `ManagedReceiveTransport` (documented at the struct level, 3 sites);
  libsrt startup failure on `Socket::connect_with` /
  `Listener::bind_with` (2 sites). Internal `.unwrap()` /
  `debug_assert!` sites that are unreachable invariants are
  intentionally not documented.

- **`tracing` instrumentation on `tst-pipeline` runtime events.**
  Sender-side reconnect attempts (target `tst_pipeline::reconnect`):
  INFO per attempt with `attempt#` / `max_attempts` / `backoff_ms`;
  DEBUG on non-zero backoff sleep; WARN on terminal give-up.
  Receiver-side reconnect (target
  `tst_pipeline::managed_receive`): mirrors the sender shape.
  `MuxSender` back-pressure threshold crossings: WARN on first
  crossing of 80% (approaching cap) and 100% (cap reached, push will
  return `BufferFull`); recovery transitions are silent.
  Per-shell lifetime spans: `info_span!` opened in `new()`, entered
  on `Drop`, target `tst_pipeline::*`. Per-call `trace_span!`
  deferred to Phase 4 perf-measurement work.

- **`Muxer::pending_packets()` and `::capacity_packets()` accessors**
  on `tst_core::mpegts::mux::Muxer`. Needed by the back-pressure
  threshold-crossing instrumentation above to compute the
  pending-vs-capacity ratio without wiring an extra field through
  `MuxerStats`.

- **README "Hello, MPEG-TS" snippet above the fold.** ~20-line
  copy-paste-runnable that exercises the muxer shape without needing
  an SRT peer or a 2-terminal setup. Cross-references
  `docs/getting-started.md` for the SRT-side walkthrough.

- **Cookbook section grouping (30 recipes → 7 sections + ToC).** The
  flat list of recipes is grouped under Sending / Muxing / Receiving
  / KLV metadata / Pairing video + KLV / Codec parsing / Operations
  with anchor-linked table of contents. Recipe numbers stay stable
  (every existing inbound link is preserved). New recipe 0:
  minimal-shape "Send a single TS packet to any Transport" using
  `RawSender` + an in-memory `Sink`.

- **Tracing quick-start in `getting-started.md`.** Copy-paste
  subscriber wiring + `RUST_LOG` filter target reference table
  documenting all the targets introduced in sub-phase 2.4.

#### Changed (Phase 2)

- **`tst-srt::init` migrated from `log` to `tracing`.** Single
  facade workspace-wide. libsrt-internal syslog levels now flow into
  `tracing` macros with `target="srt"` matching the existing
  `tst_core::*` targets — consumers wire one tracing-subscriber.

- **`tst-srt`: dropped `doctest = false`.** The lib.rs
  `SocketBuilder` quick-start is now compile-checked via `cargo test
  --doc`. Snippet rewritten with `no_run` + hidden `main` wrapper so
  CI doesn't need an SRT peer.

- **`tst-pipeline`: dropped unused `log` dep, added `tracing` dep.**

- **3 thin examples retrofitted to the rich-comment convention.**
  `mux_to_file.rs` / `pipeline_send_to_socket.rs` /
  `ts_relay_from_file.rs` — each now opens with a header banner and
  comments the why behind every non-obvious choice (PID assignments,
  PCR cadence, latency knob, NAL header byte layout, key-frame
  semantics, drain-loop pattern, `metadata_service_id` no-op for
  PrivateData, cancel-first close). Density matches
  `mux_h265_with_klv.rs`. The CLAUDE.md self-flag is now resolved.

- **Cookbook recipes 24 / 29 / 30** (`pair_klv_pipeline`,
  `parse_audio_frames`, `decode_vmti_metadata`) and **15 / 20 / 21**
  swept for consistency: `cargo run --example` invocations now
  include `-p <crate>` qualifiers (brittle to future workspace
  layout changes without). Top-of-file "Run any example with" line
  rewritten to point readers at per-recipe Runnable lines.

- **23 broken intra-doc links fixed workspace-wide** to clear the
  `cargo doc -D warnings` rail. Categories: 4 wrong-scope
  qualifications (use `[..][Self::..]` / `[..][crate::..]` form),
  1 renamed/moved item, 4 cross-crate references converted to
  inline code (cannot link upward across the crate graph), 5
  square-bracket text mistaken for links escaped with backticks
  (`buf[0]`, `programs[0]`, `Box<T>`), 4 `KlvStreamType` wrong-scope
  qualifications in `tst-pipeline`, 3 self-method backtick forms.
  `tstrans.h` regenerated to match the rustdoc edits cbindgen
  propagates into the C header.

#### Breaking (Phase 2)

- **`tst-srt`: optional `log` Cargo feature removed.** Replaced by
  unconditional `tracing` facade. Consumers wiring `log` should
  switch to `tracing-log` for compatibility.

- **4 pipeline shells gained explicit `Drop` impls.** `RawSender`,
  `DemuxReceiver`, `Receiver`, and `RawReceiver` previously had no
  `Drop` impl. The new lifetime-span instrumentation requires one
  to enter the span on shutdown. `Sender` and `MuxSender` already
  had `Drop` and are unaffected.

- **Auto-trait propagation preserved via `AssertUnwindSafe<Span>`
  wrapper.** The new private `_span: Span` field on the four shells
  above contains a `Mutex` internally, which would have flipped
  them from `RefUnwindSafe` / `UnwindSafe` to `!RefUnwindSafe` /
  `!UnwindSafe`. The wrapper preserves consumer auto-traits at zero
  runtime cost (Span is only entered in `new()` and `Drop`, never
  hot-path). `MuxSender` stays `!RefUnwindSafe` from its existing
  `Mutex<Inner<T>>`; `DemuxReceiver` stays `!RefUnwindSafe` from
  its inner `Demuxer`.

---

---

### Phase 1 — SemVer ratchet (2026-05-08)

#### Breaking (Phase 1)

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

#### Internal (Phase 1)

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

#### CI (Phase 1)

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
