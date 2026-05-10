# Changelog

All notable changes to this project are documented in this file.

Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) style.
Versioning: [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## Unreleased

Phase 1 (SemVer ratchet), Phase 2 (DX + observability), Phase 3
(FFI-readiness), and Phase 4 (performance hot paths) of the Rust quality
+ DX + FFI refactor. Plan #39 (examples reorganization) also rides this
release.

---

### Phase 4 — Performance hot paths (2026-05-10)

Bench-driven receiver + sender optimizations. 6 bench targets / 21
sub-benches established (Tasks 1–7); 5 optimization candidates committed
(Tasks 8–10, 12–13); 3 dropped per decision rule (Tasks 11, 14, 15).

#### Added (Phase 4)

- **`Demuxer::feed_aligned(&[u8; 188]) -> Result<...>`** — fast path
  that skips the sync-search buffer entirely when the caller guarantees
  the input is already a valid 188-byte packet aligned on the 0x47 sync
  byte. `DemuxReceiver` is wired to use this path internally.
  Eliminates the slice-copy-into-sync-buf round-trip on the common
  in-sync case. **-12 to -13%** on the `demux_feed_per_188` bench.

#### Performance (Phase 4)

- **Syncer ring buffer** (`tst-pipeline::pipeline::syncer`): replaced
  `buf: Vec<u8>` + `to_vec() + drain()` per-packet pattern with a
  hand-rolled ring (`head: usize` cursor, compaction at the 1316-byte
  SRT-datagram threshold). `Receiver::next_packet` now returns
  `[u8; 188]` by value (drops the `try_into().unwrap()` at call sites).
  **-60%** on `syncer_aligned_steady_1000`. (Audited estimate was 2–4%;
  the measured gain reflects that the original path paid both a heap
  allocation and a full memmove of the trailing buffer on every emit.)

- **`pid_to_program` HashMap** (`tst-core::mpegts::demux`): replaces
  the O(programs × streams) linear scan in `program_number_for_pid`
  with a `HashMap<u16, u16>` populated at PMT-handle time and cleared
  on PAT version bumps. **-9 to -10%** on `demux_feed_per_188`.

- **Muxer `pes_scratch` field**: single `Vec<u8>` reused across the 4
  PES build sites (audio / subtitle / video / KLV) instead of separate
  `Vec::with_capacity` per call. Drops 4 heap allocations per AU.
  **-5 to -8%** on `mux_end_to_end_30frames`, **-5 to -10%** on
  `push_klv_1kb`.

- **Continuity counters flat array**: `BTreeMap<u16, u8>` continuity
  counter table replaced with `Box<[u8; 8192]>` indexed by the 13-bit
  PID field; 4-bit CC masking retained. Drops the now-stale "≤4 PIDs"
  comment that rationalized the original map. **-6 to -11%** on
  `mux_end_to_end_30frames`.

- **Dropped: PMT cache + CC patch** (Task 11): -1.3% / -4.5% across
  two passes — both below the 5% host-noise-adjusted threshold;
  invalidation surface area not justified by gain.

- **Dropped: adaptation-field stuffing `fill(0xFF)`** (Task 14):
  codegen inspection showed LLVM already lowers the existing
  `for byte in &mut out[..] { *byte = 0xFF }` loop to `memset@plt`.
  No change needed.

- **Dropped: profile-guided `#[inline]` sweep** (Task 15): `perf` not
  installed on the build host; profiling was blocked. Zero `#[inline]`
  additions — valid outcome per plan.

---

### Phase 3 — FFI-readiness (2026-05-09)

Final phase of the quality + DX + FFI refactor. Six sub-phases:
3.1 (pipeline shell aliases + `Box<dyn>` blanket impls + binding-author
starter doc), 3.2 (audio frame `Owned` siblings + AV1 panic
inventory), 3.3 (stream handle opacity), 3.4 (builder reshape to
`&mut self -> &mut Self`), 3.5 (targeted API reshape: `PairerOptions`,
`usize → u64`, `CancelHandle` relocation), 3.6 (visibility + close
contracts + Rust↔C ABI cross-references + three CI ratchets).

#### Added (Phase 3 / sub-phase 3.1 — pipeline shell aliases)

- **Six `BoxedXxx` dyn-erased aliases** in `tst-pipeline` for binding
  generators (UniFFI / JNI / PyO3) that need a single concrete type
  per shell shape regardless of the underlying transport:
  `BoxedMuxSender`, `BoxedSender`, `BoxedRawSender`,
  `BoxedDemuxReceiver`, `BoxedReceiver`, `BoxedRawReceiver`. Collected
  into the new `tst_pipeline::dyn_aliases` module with crate-root
  re-exports.
- **Blanket `Transport` and `RecvTransport` impls for `Box<T: ?Sized>`**
  in `tst-core/src/transport.rs`. Without these, `Box<dyn Transport>`
  doesn't satisfy `T: Transport`, making the dyn-erased aliases
  type-level landmines. Added mid-execution after the Phase 3 plan was
  found to miss this prerequisite.
- [`docs/binding-authors.md`](./docs/binding-authors.md) — ~150-line
  starter guide for `srt-jni` / `srt-uniffi` / `tst-pyo3` authors.
  Worked Kotlin/Swift/Python/C examples plus builder + cancel-handle +
  threading + versioning sections.

#### Added (Phase 3 / sub-phase 3.2 — audio frame `Owned` siblings)

- **`codec::aac::AdtsFrameOwned`** — 11-field owned mirror of
  `AdtsFrame<'a>` with symmetric `to_owned()` / `as_ref()` round-trip.
  FFI-shaped collect-pattern doctest verifies the borrow→own→reborrow
  cycle works for binding consumers that need to retain frames
  across calls.
- **`codec::mpegaudio::FrameOwned`** — 10-field owned mirror of
  `mpegaudio::Frame<'a>` with the same symmetric round-trip and
  doctest pattern.
- **AV1 panic inventory was clean** — Phase 0 had already done the
  hardening (35 panic-shaped sites, all in `#[cfg(test)]` blocks).
  Closed Phase 0 deferral 3.2f with 10 regression tests at
  `tests/av1_no_panic.rs` exercising the production paths under
  truncation / oversized-leb128 / bit-overflow inputs.

#### Changed (Phase 3 / sub-phase 3.3 — stream handle opacity)

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

#### Changed (Phase 3 / sub-phase 3.4 — builder reshape)

- **Breaking:** Every public builder converted to `&mut self -> &mut Self`
  chainable shape:
  - `MuxerConfigBuilder` (all methods + `build()` → `&self`)
  - `MuxerProgramConfigBuilder` (all methods + `build()` → `&self`); also
    restructured to be standalone (no longer owns parent),
    `MuxerConfigBuilder::add_program` now takes a `MuxerProgramConfig`
    value, `end_program()` removed
  - `SocketBuilder` (all methods + `connect()` / `config()` → `&self`);
    `try_stream_id` now `Result<&mut Self, _>`
  - `ListenerBuilder` (all methods + `bind()` / `config()` → `&self`)
  - Descriptor-setter methods (`stream_descriptors_for_video`/`klv`/
    `audio`/`subtitle`/`stream`) on `MuxerProgramConfigBuilder` switched
    from deferred-error semantics to immediate-error
    `Result<&mut Self, MuxError>`.

  Migration: replace `Builder::new().method(x).method(y).build()` with
  `let mut b = Builder::new(); b.method(x); b.method(y); b.build()`.
  For `MuxerProgramConfigBuilder`: build the program standalone, then
  pass the value:
  `let prog = { let mut p = MuxerProgramConfigBuilder::new(num, pid); p.add_video(...); p.build() }; b.add_program(prog);`.

  Rationale: closes audit theme H5; required for clean Kotlin
  `.apply { }`, Swift `var b`, Java chaining, Python step-wise, and C
  opaque-handle binding patterns. See
  [`docs/binding-authors.md`](./docs/binding-authors.md).

#### Changed (Phase 3 / sub-phase 3.5 — targeted API reshape)

- **Breaking:** `Pairer::nearest_pts(video_pid, klv_pid, tolerance_ticks,
  max_klv_history, mode)` removed. Use
  `Pairer::with_options(video_pid, klv_pid, PairerOptions { ... })`
  instead — field-style construction with explicit `Duration` units
  composes cleanly across binding languages.
- **Breaking:** `Pairer::last_before_pts`: the third argument changed
  from `Option<i64>` (90 kHz ticks) to `Option<Duration>`. Same upgrade
  rationale: explicit units, idiomatic across language boundaries.
- **Breaking:** `MatchMode` enum removed. `PairerMode` is the path
  forward (`Realtime` and `Buffered { max_lag: Duration }`); marked
  `#[non_exhaustive]` so future variants don't break the SemVer ratchet.
- **Breaking:** `MuxError::BufferFull::capacity_packets`: `usize` →
  `u64`. JNI / UniFFI / cbindgen don't have a stable mapping for `usize`
  (32-bit on 32-bit targets, 64-bit on 64-bit targets); `u64` is
  unambiguous across architectures.
- **Breaking:** `Muxer::pending_packets()` and
  `Muxer::capacity_packets()` return `u64` (were `usize`).
- **Breaking:** `TsFramingError::SyncLost::offset` and
  `TsFramingError::NoSyncAfterLimit::max`: `usize` → `u64`.
- **Breaking:** `CancelHandle` type relocated from `tst-srt` to
  `tst-core`. The pipeline-layer cancel mechanism now lives in
  `tst_core::cancel`; `tst-pipeline` and `tst-srt` re-export it as
  `tst_pipeline::CancelHandle` and `tst_srt::CancelHandle` so binding
  authors have a single import path. Removes the libsrt-drag concern
  from the no-SRT `tst-pipeline` crate while preserving the established
  import sites.

#### Added (Phase 3 / sub-phase 3.5)

- `PairerOptions` struct (`#[non_exhaustive]`) — field-style
  construction with explicit `Duration` units; `Default` impl exposes
  the previous defaults from `Pairer::nearest_pts`.
- `Pairer::with_options(video_pid, klv_pid, PairerOptions)` — the
  replacement constructor for the removed `nearest_pts`.
- `tst_pipeline::CancelHandle` and `tst_srt::CancelHandle` re-exports —
  single import path for binding authors regardless of which crate they
  pull in.
- [`docs/cancel-handle.md`](./docs/cancel-handle.md) — universal
  cross-thread shutdown pattern with per-language idiom table
  (Kotlin `Job.cancel()`, Swift `Task.cancel()`, Python
  `threading.Event`, C `tst_cancel_handle_cancel`).
- Cookbook recipe (Operations section): graceful shutdown from another
  thread via `CancelHandle`.
- Architecture doc section: cross-thread shutdown via `CancelHandle`.

#### Changed (Phase 3 / sub-phase 3.6 — visibility + close contracts)

- **`klv::pack::Iter` is now `#[doc(hidden)]`.** The iterator is still
  `pub` (a downstream fuzz target depends on it — see Phase 1 Task
  1.3.4 deferral), but it no longer appears in rustdoc. Public iteration
  over KLV packs goes through `klv::pack::iter()` and the typed
  `klv::st0601` / `klv::st0102` / `klv::st0903` decoders.

#### Added (Phase 3 / sub-phase 3.6)

- **Close-contract rustdoc on 11 long-lived public types.** Each type
  now carries a `# Closing` section spelling out the resource-cleanup
  contract for binding authors, plus a per-language idiom table
  covering Rust `Drop`, Kotlin `use { }` / `AutoCloseable`, Swift
  `defer`, Python `__exit__` / `with`, and C explicit-free pairing.
  Coverage: `MuxSender`, `Sender`, `RawSender`, `DemuxReceiver`,
  `Receiver`, `RawReceiver`, `Pairer`, `Socket`, `Listener`, `Muxer`,
  `Demuxer`. (Tasks 3.6.2 + 3.6.3.)
- **Rust ↔ C ABI cross-references on public methods.** Sender shells'
  10 most-used methods (Tasks 3.6.4) plus tst-srt and tst-core public
  methods (Task 3.6.5) now carry `# C ABI` rustdoc sections naming the
  matching `tst_*` C entry point, and the C header carries reverse
  references to the Rust path. Binding authors no longer need to
  hand-trace the mapping.
- **Three new CI ratchets** under `scripts/`:
  - `check-c-abi-rustdoc-coverage.sh` (Task 3.6.6) — verifies every
    `tst_*` C ABI export has a matching `# C ABI` rustdoc reference
    and vice-versa. Bidirectional: catches both Rust→C drift (new C
    entry point not surfaced in Rust docs) and C→Rust drift (new Rust
    method not annotated). Currently locks in 74 C ABI exports.
  - `check-close-contract-presence.sh` (Task 3.6.7) — verifies all
    11 long-lived public types still carry a `# Closing` rustdoc
    section. Catches accidental removals during refactors.
  - `check-no-public-usize.sh` (Task 3.6.8) — guards against `usize`
    sneaking back into the public API surface. Sub-phase 3.5
    eliminated all public `usize` (replaced with `u64` for FFI
    portability); this keeps the surface clean.

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
