# Changelog

All notable changes to this project are documented in this file.

Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) style.
Versioning: [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [Unreleased] — tst-py: Python bindings audit small wave (2026-05-24)

Small carry-forward batch from the 2026-05-24 Python bindings audit
(`docs/python-1/python-bindings-audit.md`). Three independent fixes
shipped as three commits; the larger #3 + #5-14 backlog gets its own
plan.

**CI:**

- New `python-core` GitHub Actions job runs the default (non-pandas)
  pytest suite after `maturin develop --release`. Closes audit #1
  (Python core tests previously had no CI coverage — only the pandas
  extra job ran, and only against pandas-marked tests).
- The 12th bash ratchet `scripts/check-py-codec-error-mapping-coverage.sh`
  now runs in CI (as the first step of `python-core`). Future
  `CodecParseError` variants can no longer silently miss Python
  exception mapping.

**Tests:**

- Pandas-marked test modules now use `pytest.importorskip("pandas")`
  / `pytest.importorskip("numpy")` so the default `-m 'not pandas'`
  run can collect without the optional extras installed. Latent bug
  surfaced by the new `python-core` job — collection failed on
  `import pandas as pd` in the 4 `test_pandas_*.py` files when the
  extras weren't installed.

## [Unreleased] — mpegts: multi-cell AU cell reassembly (2026-05-24)

**`mpegts::demux` now reassembles fragmented Metadata AU cells per
H.222.0 V9 §2.12.4.2.** Two flavors covered: multiple AU cells back-to-back
within one PES (previously silently truncated to the first cell), and cells
of a single AU spread across multiple PES packets (`First` / `Middle` /
`Last`). Per-PID reassembly buffer capped at 1 MiB by default; failure
modes surface as typed `MultiCellAuReason` on
`NonConformantIssue::MultiCellAu`.

**Added:**

- `mpegts::demux` multi-cell Metadata AU cell reassembly
  (H.222.0 §2.12.4.2). Fragmented sync-metadata AUs (`First` / `Middle` /
  `Last`) now reassemble into a single demuxer event with
  `MetadataKind::KlvSyncAuCell::was_reassembled = true` and
  `cell_count = N`. Multiple AU cells packed into one PES (previously
  silently truncated to the first cell) now each emit their own event.
- `DemuxerConfig::au_cell_cap_per_pid` (default 1 MiB) caps the per-PID
  reassembly buffer; overflow drops with
  `NonConformantIssue::MultiCellAu { reason: Overflow, .. }`.
- `MultiCellAuReason` enum (`Orphan` / `SequenceGap` / `ConcurrentFirst` /
  `Overflow`) surfaces typed reassembly failure modes via
  `NonConformantIssue::MultiCellAu::reason`.
- `MetadataKind::KlvSyncAuCell` gains `was_reassembled: bool` +
  `cell_count: u32` fields.

**Changed:**

- `NonConformantIssue::MultiCellAu` Display reworded — describes the
  reassembly failure mode instead of the prior "not implemented" message.

**Bindings:**

- `tst-py`: `_KlvEvent` gains `was_reassembled: bool` + `cell_count: int`;
  new `MultiCellAuReason` PyO3 `eq_int` enum (`ORPHAN` / `SEQUENCE_GAP` /
  `CONCURRENT_FIRST` / `OVERFLOW`); `_NonConformantEvent` gains optional
  `multi_cell_au_reason` field.
- `tst-c`: `TstEventMetadata` gains `was_reassembled` + `cell_count` at
  the end of the struct (additive ABI change); new
  `tst_multi_cell_au_reason` enum mirrors `MultiCellAuReason`;
  `TST_ABI_VERSION_MINOR` bumped 2 → 3.

**Corpus validation (2026-05-24, 251 files):**

- _FMV captures (28 files): **60,089 typed KLV records** (previously 0
  before this plan — every cell past the first was silently dropped).
  175,121 NonConformant `MultiCellAu(Orphan)` events surface real
  wire-format issues that were previously invisible.
- _Raw captures (22 files): 88,635 typed KLV records, 92,691 Orphans.
- Other captures (201 files): 1,644,958 typed KLV records, 113,355
  Orphans.
- Total across 251 files: **1,793,682 typed KLV records.**
  `klv_reassembled = 0` indicates the corpus's actual fragmentation
  pattern is back-to-back Complete cells in one PES (the
  classify_klv-only-looked-at-first-cell bug) plus malformed
  `Middle`/`Last`-CFI orphans — not legitimate `First`+`Middle`+`Last`
  cross-PES fragmentation (which Task 5 integration tests prove the
  reassembler handles correctly).

---

## [Unreleased] — tst-py Phase 6 — pandas + NumPy adapters (2026-05-23)

**`tstrans.pandas` DataFrame adapters + zero-copy NumPy views via optional
[pandas] extra.** Python-only; no Rust changes. Existing `pip install
tstrans` workflow continues to work identically — adapters and accessors
appear only when `pip install 'tstrans[pandas]'` activates the extra.

**Added:**
- Optional `[pandas]` extra in `pyproject.toml` (pandas >= 2.0, numpy >= 1.24).
  Install: `pip install 'tstrans[pandas]'`. Without the extra, the
  adapters raise a friendly `ImportError` directing the user to install
  the extra.
- New `tstrans.pandas` submodule with 5 DataFrame adapters:
  - `klv_to_dataframe(records, *, mode="summary")` — polymorphic over
    `UasDatalinkLs` / `SecurityLs` / `PrecisionTimeStampPack` / `VmtiLs`.
    KLV DataFrames use `pd.DatetimeIndex(tz="UTC")` built from
    `timestamp_us` (microseconds since UTC epoch) when present; RangeIndex
    fallback when absent. ST 0903 supports `mode="summary"` (one row per
    VMTI LS) and `mode="targets"` (MultiIndex `[pts, target_id]`, one row
    per VTarget). Composite fields flatten to FLAT scalar columns (e.g.
    `frame_center_lat_deg`, `corner_lat_pt1_deg`).
  - `events_to_dataframe(events)` — union schema across all DemuxEvent
    kinds. Kind labels: `Pmt` / `ProgramMap` / `Sample` / `Klv` /
    `Discontinuity` / `NonConformant` / `EndOfStream`.
  - `nals_to_dataframe(nals, pts=None)` — H.264/H.265/H.266 NAL lists
    with spec-name lookup column.
  - `obus_to_dataframe(obus, pts=None)` — AV1 OBU lists with type-name
    lookup.
  - `audio_frames_to_dataframe(frames)` — polymorphic AdtsFrame /
    Mpeg2AudioFrame; enum-typed fields render as bare names (e.g. `LC`,
    `III`, `JOINT_STEREO`) for analyst-friendly grouping.
- Zero-copy NumPy accessors via Python monkey-patches on 15 byte-bearing
  classes:
  - `.payload_np` on `NalUnit` / `Obu` / `AdtsFrame` / `Mpeg2AudioFrame`.
  - `.raw_rbsp_np` on H.264/H.265/H.266 `Sps` / `Pps` / `Vps` /
    `SliceHeaderLight`.
  - `.raw_np` on `Av1SequenceHeader` / `Av1FrameHeaderLight`.
  - Returns `numpy.ndarray` (uint8 view) backed by the underlying `bytes`
    — no copy.
- `KlvFieldError` `field_errors` field renders as `|`-joined string of
  `tag<N>:<kind>:<message>` triples in the DataFrame (pipe-separator
  avoids ambiguity since `KlvFieldError.__str__` already contains commas).
- `nal_count` column is gated to video-only event rows; audio AdtsFrame
  lists do not populate it (avoids misleading `df.nal_count > N` filters
  on audio sample rows).
- Pytest marker `@pytest.mark.pandas` (default-skipped via
  `addopts = "-m 'not pandas'"` in `pyproject.toml`). Existing 513
  default unit tests remain unchanged.
- New CI job `python-pandas-extra` installs the extra and runs
  marker-only pytest (`pytest -m pandas --override-ini=addopts=`).
- 7 `test_pandas_missing_extra.py` tests (NOT marked) confirm every
  adapter raises a friendly `ImportError` when the extra is absent.
- User guide at `ts-transformer/docs/guide-python-pandas.md` covers
  install, adapters, NumPy views, common analyst recipes.

**Unchanged:**
- Core package `pip install tstrans` continues to work identically to
  Phase 5; no return types change.
- BASELINE non_exhaustive: 159 (Phase 6 = Python-only, no Rust changes).
- 12 bash ratchets: green.
- All 3 cargo public-api baselines (`tst-core` / `tst-pipeline` /
  `tst-srt`): clean.

**Internal:**
- pytest count: 513 → 520 default (added 7 missing-extra tests) +
  60 pandas-marker = ~582 total (520 default + 60 pandas, plus 5
  skipped across the two runs).
- No new Rust public API surface.
- No new fuzz targets.

---

## [Unreleased] — tst-py Phase 5 — Codec parsers (2026-05-23)

**`tstrans.codec` module fully populated; `Sample.payload` typed-replaced.**
(`b8aa957..a9d1634`). Exposes ~50 classes and ~25 functions covering
H.264 / H.265 / H.266 / AV1 / AAC / MPEG-2 audio parsers. Three new
Rust parsers (`parse_slice_header_light` for H.264 / H.265 / H.266) and
three fuzz harnesses ship alongside the Python surface. `Sample.payload`
changes from `bytes` to a typed list — first breaking change since Phase 2
(pre-1.0, per the project's break-freely policy).

**Added:**

- `tstrans.codec` module: `NalUnit`, `Obu`, `ObuExtension`,
  `H264Sps`, `H264SliceHeaderLight`, `H264SliceType`, `H264Pps`,
  `H264HrdParameters`, `H264VuiParameters`, `H264ColorPrimaries`,
  `H264MatrixCoefficients`, `H264TransferCharacteristics`,
  `parse_h264_sps`, `parse_h264_slice_header_light`,
  `H265Sps`, `H265SliceHeaderLight`, `H265SliceType`, `H265Vps`,
  `H265Pps`, `H265HrdParameters`, `H265VuiParameters`,
  `H265ColorPrimaries`, `H265MatrixCoefficients`,
  `H265TransferCharacteristics`, `parse_h265_sps`,
  `parse_h265_slice_header_light`,
  `H266Sps`, `H266SliceHeaderLight`, `H266SliceType`,
  `parse_h266_sps`, `parse_h266_slice_header_light`,
  `Av1SequenceHeader`, `Av1FrameHeaderLight`, `Av1FrameType`,
  `ObuType`, `parse_av1_sequence_header`,
  `parse_av1_frame_header_light`,
  `AdtsFrame`, `AacProfile`, `parse_adts_frame`,
  `Mpeg2AudioFrame`, `Mpeg2AudioVersion`, `Mpeg2AudioLayer`,
  `Mpeg2AudioChannelMode`, `parse_mpeg2_audio_frame`.
- New Rust public APIs: `tst_core::codec::{h264,h265,h266}::parse_slice_header_light`
  + `*SliceHeaderLight` structs + `*SliceType` enums (parity with AV1's
  existing `parse_frame_header_light`). H.266 returns sentinel `slice_type`
  / `pps_id` (deferred — full VVC syntax deferred per plan Task 4 note).
- 3 new fuzz harnesses in `crates/tst-core/fuzz/fuzz_targets/`
  (`fuzz_h264_slice_header_light`, `fuzz_h265_slice_header_light`,
  `fuzz_h266_slice_header_light`).
- `tstrans.exceptions.CodecError` + `CodecErrorKind` enum — 8-variant
  error hierarchy matching Rust `CodecParseError`.
- 12th bash ratchet `scripts/check-py-codec-error-mapping-coverage.sh`.

**Changed (BREAKING — pre-1.0):**

- `tstrans.mpegts.Sample.payload` changed type from `bytes` to one of
  `list[NalUnit]` (H.264 / H.265 / H.266) / `list[Obu]` (AV1) /
  `list[AdtsFrame]` (AAC) / `list[Mpeg2AudioFrame]` (MPEG-2 audio).
  Subtitle and AAC-LATM remain `bytes`. On audio parse failure mid-stream,
  payload falls back to `bytes` and `sample.codec_parse_error: CodecError`
  is populated.

**Internal:**

- `BASELINE` non_exhaustive count: 140 → 159 in `.github/workflows/ci.yml`.
- `tst-core` public-api baseline updated (+19 new entries for
  `*SliceHeaderLight` / `*SliceType` / `parse_slice_header_light` × 3 codecs).
- pytest count: 270 → 513 (2 skipped throughout).

---

## [Unreleased] — tst-py Phase 4 — Muxer wrap + KLV encoders (2026-05-23)

**Python bindings build path complete via 15 subagent-driven tasks**
(`8912bf5..aa09777`). Wraps the full `tst_core::mpegts::mux::Muxer`
surface (config family + push entries + handles + stats + draining
context-manager sink) and adds symmetric KLV encoders for ST 0601 /
0102 / 0605 / 0903. After Phase 4, a notebook can parse a `.ts` file
(Phase 2), modify KLV records (Phase 3), and re-mux to a new `.ts`
(Phase 4) — closing the round-trip use case from the parent spec.

**Added:**

- `tstrans.mpegts.{Muxer, MuxerConfig, MuxerConfigBuilder,
  MuxerProgramConfig, MuxerProgramConfigBuilder}` — 4-type config
  family mirroring Rust 1:1.
- `tstrans.mpegts.{KlvStreamType, Av1CarriageMode}` — pure-Python
  enums (`KlvStreamType.SYNCHRONOUS_METADATA / PRIVATE_DATA`;
  `Av1CarriageMode.MPEG2_TS_BINDING / INTEROP_RAW_OBU`, default
  `MPEG2_TS_BINDING`).
- `tstrans.mpegts.{StreamSpec, VideoStreamSpec, KlvStreamSpec,
  AudioStreamSpec, SubtitleStreamSpec}` — frozen dataclass tagged
  union over the streams in a program; Python 3.10+ match-statement
  compatible.
- `tstrans.mpegts.{VideoStreamHandle, AudioStreamHandle,
  KlvStreamHandle, SubtitleStreamHandle}` — opaque `u32`-backed
  PyO3 newtypes for handle-form pushes.
- `tstrans.mpegts.{MuxerStats, StreamCodecStats,
  VideoStreamCodecStats, KlvStreamCodecStats,
  AudioStreamCodecStats}` — stats accessors + per-stream tagged
  union (`Unknown` collapses to `None`).
- `tstrans.mpegts.{MuxerFileSink, MuxerDrainProxy}` +
  `Muxer.write_file(path)` — context manager that auto-drains
  after each push and flushes-and-closes on `__exit__`, never
  suppresses user exceptions.
- `tstrans.klv.{encode_uas_datalink,
  encode_uas_datalink_strict_compliance}` — ST 0601 encoders.
  `_strict_compliance` raises
  `KlvEncodeError(MISSING_MANDATORY_ITEM)` per ST 0601.8 §10.3.
- `tstrans.klv.encode_security` — ST 0102 encoder.
- `tstrans.klv.encode_precision_timestamp` — ST 0605 encoder
  (always returns 26 bytes).
- `tstrans.klv.{encode_vmti, encode_vmti_standalone}` — ST 0903
  encoders; `encode_vmti` emits LS body only,
  `encode_vmti_standalone` adds the SMPTE UL + BER-length prefix.
  `parse_klv_universal(encode_vmti_standalone(rec))` round-trips.
- `tstrans.exceptions.{MuxError, MuxErrorKind, KlvEncodeError,
  KlvEncodeErrorKind}` — 5-variant `MuxErrorKind` (`INPUT_MALFORMED
  / CONFIG_INVALID / INVALID_USAGE / BACKPRESSURE / INTERNAL`);
  8-variant `KlvEncodeErrorKind`.

**Push surface conventions (mirrors Rust 1:1 — arg order is
deliberately non-uniform across stream kinds):**

- `push_video(nal, pts, key_frame=False)`,
  `push_video_to(handle, nal, pts, key_frame=False)`,
  `push_video_to_with_dts(handle, nal, *, pts, dts, key_frame=False)`.
  No `pid` parameter — auto-resolves the lone video stream or
  raises `MuxError(INVALID_USAGE)` (Rust `AmbiguousTarget`).
- `push_audio(frames, pts)`,
  `push_audio_to(handle, pts, frames)`. **Arg order differs.**
- `push_klv(klv, pts, metadata_service_id=0)`,
  `push_klv_to(handle, klv, pts, metadata_service_id=0)`.
- `push_subtitle(pts, payload)`,
  `push_subtitle_to(handle, pts, payload)`. **Pts before payload.**
- Unknown handle → `MuxError(INVALID_USAGE)`; invalid payload →
  `MuxError(INPUT_MALFORMED)`; back-pressure →
  `MuxError(BACKPRESSURE)`.

**Verified:**

- Determinism sentinel: two fresh `Muxer` instances with the same
  config + same push sequence emit byte-identical output (4324
  bytes / 23 packets for the synthetic 1-video-1-klv fixture).
- 5/5 video + 5/5 klv input frames round-trip to 5 video + 5 klv
  events via `parse_file` (exact, not just within tolerance).
- 270 pytest tests + 2 skipped (was 178 after Phase 3 cleanup
  → +92 tests).

**Known follow-ups (NOT blocking Phase 5):**

- `add_subtitle` raises `NotImplementedError` — mux-side
  `SubtitleCodec` is a struct-variant Rust enum (per-variant
  fields for language / subtitling-type) incompatible with the
  Phase 2 flat `SubtitleCodec` Python string-enum. Deepening the
  Python representation is the work item.
- Full real-fixture round-trip needs config-from-probe
  reconstruction (currently the `tests/fixtures/local/`
  smoke-test is `pytest.skip`).
- NumPy zero-copy view of `pull()` output (Phase 6 pandas extra).
- `Muxer.write_to(io.BufferedWriter)` second sink shape (if a
  consumer asks).

**Build infrastructure:**

- BASELINE `#[non_exhaustive]` count bumped 135 → 140 in
  `.github/workflows/ci.yml`.

---

## [Unreleased] — Validate-1 act-now batch (plan #94, docs/plans/2026-05-22-validate-1-act-now-batch.md)

**Ten validate-1 carry-forward items closed via 7-worktree parallel SDD,
2026-05-22.** Shipped as 7 cherry-picks on `main` (`707f447..dc3a4b6`);
the other two of the act-now-12 list were verified-superseded at pre-flight
(ST0903-02 by Sprint 3 E5, M-05 by Sprint 2 B8). H264-RV2 was found
pre-shipped by Sprint 1 A8 `00bd703` during execution.

**Fixed:**

- **IMAPB `min < max` precondition guard** (commit `f36d533`, M-02). Encode
  and decode now reject `min >= max` up front with new variants
  `KlvEncodeError::InvalidImapbParams` and `KlvFieldError::InvalidImapbParams`
  rather than returning misleading bounds errors deeper in the call.
- **ST 0601 `encode_strict_compliance`** (commit `f36d533`, ST0601-NEW-01).
  Mirrors `decode_strict_compliance` on the encode side; new variant
  `KlvEncodeError::MissingMandatoryItem { tag, reason }` so producers can
  validate against ST 0601.8 §10.3 before emitting wire bytes.
- **Null PID skip in `cc_by_pid` tracking** (commit `656cecf`, Slice 06 M-02).
  Stuffing/null-PID packets no longer pollute the continuity-counter state
  machine, eliminating spurious `ContinuityJump` events under sparse PSI.
- **H.264 constraint flags consulted for `profile_idc=100` B-frame detection**
  (commit `dc3a4b6`, H264-RV4). `H264Sps::constraint_set_flags` is now a
  typed newtype; B-frame heuristics consult `constraint_set1_flag` per
  H.264 §A.2.1.
- **H.264 `frame_rate` `saturating_mul(2)` with None-on-saturation** (commit
  `dc3a4b6`, H264-RV7). Field-coded streams with extreme `num_units_in_tick`
  no longer panic on overflow; the iterator returns `None` for the affected
  frame and keeps walking.
- **H.265 `scaling_list` returns `EngineError` not `UnsupportedProfile`**
  (commit `46b2fd0`, H265-V1-M02). Scaling-list parse failures are
  surfaced as the parser-internal error type rather than misclassified as
  an unsupported codec profile; 3 conformance-fixture consumer sites
  updated.
- **AV1 `count_av1_obus` overflow safety** (commit `4b856a3`, TC-ROOT-V1-M2).
  `checked_add` replaces wrapping arithmetic; truncation surfaces as
  `None` rather than a wrap to zero.
- **`srt-sys` `SocketGuard` RAII for listener handle** (commit `707f447`,
  SS-V1-04). `encrypted.rs` listener tests no longer leak SRT sockets on
  early return; the guard wraps `srt_close` per-test.
- **`mock_transport` mutex poison recovery** (commit `738e9b2`, SS-V1-10).
  `unwrap_or_else(|e| e.into_inner())` lets a panicking test continue
  draining state from the shared mock instead of cascading the poison.

**Workspace updates:**

- BASELINE `#[non_exhaustive]` stays at 135 (additive variants on already-
  `#[non_exhaustive]` enums).
- `crates/tst-core/public-api.txt` baseline regenerated for
  `encode_strict_compliance` + 3 new error variants. `tst-pipeline` and
  `tst-srt` baselines unchanged.
- All 11 bash ratchets green.

**Follow-up (same day):** commit `46a454e` ported
`check-lifecycle-ffi-catch-coverage.sh` and `check-c-abi-rustdoc-coverage.sh`
to the portable `while IFS= read -r x; do arr+=("$x"); done < <(...)`
pattern (bash 3.2 / macOS), unblocking `macos-arm64` from gating
promotion (target 2026-05-30). First post-fix CI run had `macos-arm64`
pass cleanly for the first time since Sprint 3 D1.

## [Unreleased] — Validate-1 Phase 2 Sprint 4-5 follow-ups (docs/validate-1/15-sprint-4-5-review-codex.md)

**Five follow-up fixes from a 2026-05-20 Codex review of Sprints 4-5.**
Closed by 4 commits on `main` (`a0b0f8f`, `feffff8`, `361242a`, `d711ecb`).
Codex review at `docs/validate-1/15-sprint-4-5-review-codex.md`.

**Fixed:**

- **Stats wired to `frames_with_resync`** (commit `a0b0f8f`, follow-up #2).
  Sprint 4 G2 added the resync iterator but the `pes_emit` and
  `push_audio` stats sites were still calling strict `frames()`. The
  user-visible symptom Sprint 4 G2 named ("stats undercount on first
  parse error") is now actually fixed; this is the pattern documented in
  the `feedback_g2_pattern_plan_says_fix_symptom.md` memo.
- **`ManagedDemuxReceiver` data-loss budget rustdoc + no-dead-tail test**
  (commit `feffff8`, follow-up #3). Documents the reconnect drop budget
  (≤ `max_payload` bytes, typically ~7 TS packets, never an entire flow)
  and adds an integration test asserting no dead tail under repeated
  reconnect.
- **`tst-c` wires `ManagedDemuxReceiver` + `TST_EVENT_RECONNECT_DISCONTINUITY = 6`**
  (commit `361242a`, follow-up #1). New C entry points
  `tst_managed_demux_receiver_*` + new event kind exposed via the
  existing tagged-union event ABI. `TST_ABI_VERSION_MINOR` bumped 1 → 2
  (additive). Four pass-through delegates added on the managed wrapper:
  `stats`, `reset_stats`, `socket_stats`, `stream_codec_stats`.

**Workspace updates:**

- BASELINE `#[non_exhaustive]` stays at 135 (`TstEventKind` is not
  `#[non_exhaustive]` per existing convention).
- Docs-only follow-ups: 11-phase-2-plan.md Wave I SHIPPED + Sprint 5
  SHIPPED status blocks, 99-audit-summary.md H5/H6/H7/H8/H10/H11 cells
  closed, 13-interop-results.md acceptance-criterion correction.

## [Unreleased] — Validate-1 Phase 2 Sprint 5: Wave I empirical interop (docs/validate-1/11-phase-2-plan.md §2.6)

**Four empirical-interop fixtures from the Validate-1 audit's "validate
in the world" wave.** Shipped as 4 commits on `main`
(`774181c..e900779`) on 2026-05-20 via 4-worktree parallel SDD.

**Added (tests + harness only — no public-API delta):**

- **WebVTT / CEA-708 ignore-matrix** (commit `774181c`, I1). 32/32 ignore
  cells confirmed across ffprobe / tsp / tsanalyze / gst-launch tsdemux,
  empirically validating the H7 docs-only stance for these two subtitle
  codecs.
- **AV1 binding-conformant external decoder interop** (commit `ed80acd`,
  I2). ffmpeg / ffprobe accept the binding stream byte-identically to
  the legacy `Av1InteropRawObu` stream, validating the D-1 default
  (`Av1CarriageMode::Mpeg2TsBinding`).
- **KLV ST 1201.5 + ST 0903.6 spec vectors** (commit `f3fe8f9`, I3). 54
  tests covering 7/7 IMAPB substrate variants, `L ∈ 1..=8` bounds,
  Appendix A worked examples, and 22 BER-OID symmetry pairs across
  ST 0107.5 / 0601 / 0102 / 0903.
- **`scripts/cross-impl-byte-diff.sh`** (commit `e900779`, I5). 5-case
  content matrix comparing our output against tsduck `tsp` (byte-
  identical on all 5) and ffmpeg (diffs cosmetic only).

**Workspace updates:**

- BASELINE `#[non_exhaustive]` stays at 135 (tests + scripts only).
- No external receiver rejected any output under any test case.
- Plan-text gap surfaced (recorded in 13-interop-results.md): ST 0903.6
  §10.1.11/12 are Horizontal_FOV / Vertical_FOV worked examples, not
  incremental-update flows. If a different MISB doc was intended, file
  as a follow-up plan.

## [Unreleased] — Validate-1 Phase 2 Sprint 4: Waves F + G + H pipeline/codec/docs sweep (docs/validate-1/11-phase-2-plan.md §2.4 + §2.5)

**Twenty-one fixes (5 F + 3 G + 13 H docs sweep) from the Validate-1
audit's pipeline-correctness + codec-conformance + documentation
slices.** Shipped as 10 commits on `main` (`7275ae8..6182f02`) on
2026-05-20 via 8-worktree parallel SDD.

**Fixed (High):**

- **`ManagedDemuxReceiver` shell + `Demuxer::reset_sync` +
  `DemuxEvent::ReconnectDiscontinuity`** (commits `987c230` + `c969129`,
  F2; BREAKING — new public API). Sibling to `ManagedTransport` for the
  demux pipeline. On underlying-transport reconnect, the demuxer's
  syncer is reset and a `ReconnectDiscontinuity` event is emitted so
  consumers can mark a hard discontinuity in any downstream state.
  Reassembly tables (PAT/PMT, per-PID CC, last PTS) are preserved
  across reconnect.
- **`*_with_options(*Config)` → `*_with_config` rename** (commit
  `834a651`, F5; BREAKING). Workspace-wide constructor convention sweep:
  `Demuxer::with_options(DemuxerConfig)` → `Demuxer::with_config(DemuxerConfig)`,
  same pattern on `Pairer` and `io_file`. `DemuxReceiver::with_demux_options`
  is intentionally kept as a deferred rename (see `docs/conventions.md`
  outliers table).
- **`CodecParseError::UnsupportedFreeFormat` + `frames_with_resync()`**
  (commit `df51bdf`, G1 + G2). `bitrate_index == 0` (legal per ISO
  11172-3 but unsupported here) now surfaces a distinct variant rather
  than `ReservedValue`. New `frames_with_resync()` iterators on both
  `codec::mpegaudio` and `codec::aac::adts` walk past unparsable bytes
  to recover stream-wide stat accuracy.
- **`F2+F5` cross-worktree integration fix** (commit `6182f02`).
  `ManagedDemuxReceiver` was constructing the demuxer via the
  post-F5-renamed `Demuxer::with_config` after the rename landed on a
  separate worktree; integrated fix-up applied per
  `feedback_cherry_pick_build_between_parallel_worktrees.md`.

**Fixed (Medium):**

- **ADTS `profile = 3` gated on MPEG-2/-4 ID bit** (commit `12fb3d4`, G3).
  `profile = 3` is the SSR (Scalable Sample Rate) profile for MPEG-2 AAC
  but reserved for MPEG-4 AAC; the iterator now consults the `ID` bit
  and rejects vs. accepts accordingly.
- **`MuxSender::Drop` gated on `!closed`** (commit `5869573`, F4). The
  drop impl no longer double-cancels an already-closed sender; idempotent
  by construction.

**Documentation:**

- **`Transport::close` vs `RecvTransport::close` asymmetry** (commit
  `7275ae8`, F3). Rustdoc clarifies that the send-side `close` waits for
  outbound queue drain while the receive-side `close` is immediate;
  asymmetry was real but undocumented.
- **`max_unsynced_bytes` is diagnostic-only** (commit `c942d6d`, F1).
  Clarifies the threshold is a warning, not a fail-fast — a long
  unsynced run is logged but doesn't terminate the receive loop.
- **Wave H one-shot docs sweep (H1-H13)** (commit `ceb54a5`). Thirteen
  small docs corrections / consistency fixes across guides and examples;
  scrub-guard regex extended to forbid `srt-c` (which would catch a
  regression to the inner-workspace shape).

**Workspace updates:**

- BASELINE `#[non_exhaustive]` bumped 134 → 135 (one new
  `ManagedDemuxReceiverConfig` in `tst-pipeline`).
- `crates/tst-pipeline/public-api.txt` baseline regenerated (new
  `ManagedDemuxReceiver` + `ManagedDemuxReceiverConfig` + new variant on
  `DemuxEvent`).

## [Unreleased] — Validate-1 Phase 2 Sprint 1-3 review follow-ups (docs/validate-1/14-sprint-1-3-review-codex.md)

**Three follow-up fixes from a 2026-05-20 Codex review of Sprints 1-3.**
Codex re-reviewed the response and corrected the Sprint 3 BASELINE wave
attribution per `feedback_baseline_attribution_verify_via_ci_yml_diff.md`.

**Fixed:**

- **A4 bounded-PES residual-discard rationale clarified** (commit
  `c351b1f`). Rustdoc now records why tail residual is dropped along
  with per-PID state (Option B per Sprint 1 plan) rather than emitted as
  a malformed-PES diagnostic.
- **Descriptors module docs cover Result-returning builders** (commit
  `1631123`). `mpegts::descriptors` rustdoc previously documented only
  the parser path; now covers the build-side error subset.
- **AV1 per-OBU framing fix** (commit `9f83250`, extending Sprint 2 C8
  chain). The first AV1 OBU in a Frame carrier was getting the spec
  3-byte start code but subsequent OBUs were not; per-OBU framing now
  unconditionally emits the 3-byte prefix.

**Workspace updates:** BASELINE `#[non_exhaustive]` stays at 134 (no
new variants).

## [Unreleased] — Validate-1 Phase 2 Sprint 3: Waves D + E FFI hardening + KLV strict-compliance (docs/validate-1/11-phase-2-plan.md §2.4)

**Twelve FFI + KLV-encode fixes from the Validate-1 audit.** Shipped as
19 commits on `main` (`5813c72..566789b`) on 2026-05-20 via 11-worktree
3-phase parallel SDD.

**Fixed (High):**

- **C ABI lifecycle entries wrapped in `ffi_catch`** (commits `94cdfaf`
  + `feff2ff`, D1). `_close` and `_cancel` entries previously bypassed
  panic-catch; a panicking shell on close could unwind across the FFI
  boundary as undefined behaviour. New (11th) bash ratchet
  `check-lifecycle-ffi-catch-coverage.sh` ratchets this against
  regression.
- **`TransportError` carries optional typed errno source** (commits
  `76e89fa` + `4f3a1e0`, D5). `TransportError::*` now embed an
  `Option<TypedErrnoSource>` so FFI bindings can propagate the libsrt /
  POSIX errno code distinctly from the message string. New trait method
  `ShellError::errno_code()`.
- **DTS + PTS migrated to `Option<Pts90khz>`** (commits `07641e0` +
  `d169771`, D4; BREAKING). Previously both lived as `Option<i64>` on
  `PesPayload` — now typed at the API boundary.
- **`sizeof` guards for 7 `#[repr(C)]` stats structs** (commits `a3ce423`
  + `6c6c87a`, D2). New `static_assert(sizeof(struct) == EXPECTED)` per
  struct in the cbindgen-generated header, catching silent layout drift
  before bindings link.

**Fixed (KLV encode strictness — Wave E):**

- **ST 0601 strict-mode duplicate-tag + canonical BER walker** (commit
  `76361ed`, E1 + E2). Strict decode now rejects duplicate tags and
  non-canonical BER length encodings.
- **ST 0601 encode reserved-tag filter** (commit `5f6ddd9`, E3). New
  variant on the unknown-tag enum; reserved tags filtered before
  serialisation.
- **ST 0102 BER-OID encode for unknown tags** (commit `20d1038`, E4).
  Symmetric with the BER-OID decode path added in Sprint 2.
- **ST 0903 BER-OID local-set walker + `VTargetPack` inner walk**
  (commits `031b3c4` + `566789b`, E5). VMTI local sets and per-target
  packs now traverse BER-OID encoded tags correctly; nested LSes still
  pass-through.

**Fixed (other Wave D items):**

- **`SrtCancelHandle` module docstring** (commit `489eaf6`, D3). Clarifies
  that the handle is the canonical cross-thread shutdown primitive and
  that `is_cancelled()` is advisory.
- **`MockRecvTransport` with `FailMode` fixtures** (commits `1197b61` +
  `11cc28f` + `c27d2ff`, D7). New test helper with deterministic failure
  injection (Closed, Broken, Backpressure) for receive-side reconnect
  tests.
- **`srt-sys` cdylib `--exclude-libs=ALL` on Linux** (commit `5813c72`,
  D6). Symbol-hygiene fix preventing vendored mbedTLS / libstdc++
  symbols from leaking into `libtstrans.so`'s dynamic symbol table.

**Workspace updates:**

- BASELINE `#[non_exhaustive]` bumped 131 → 134 (+3: Wave D additions;
  Wave E additions ride on already-`#[non_exhaustive]` enums).
- `crates/tst-core/public-api.txt` baseline regenerated (`Pts90khz` on
  DTS, new `ShellError::errno_code` trait method, new KLV-encode
  variants, new `MockRecvTransport`).
- 11 bash ratchets (10th was added in plan #93; 11th
  `check-lifecycle-ffi-catch-coverage.sh` added here).

---

## [Unreleased] — Validate-1 Phase 2 Sprint 2: Waves B + C demux & mux conformance (docs/validate-1/11-phase-2-plan.md §2.2 + §2.3)

**Sixteen demux-correctness, mux-conformance, and FFI-hardening fixes from the
Validate-1 Phase 1 audit (Codex + Claude reports at `docs/validate-1/`).**
Shipped as 20 commits on `main` (`43545ef..2d73294`) on 2026-05-19/20 via
parallel `superpowers:subagent-driven-development` background controllers
across 13 isolated git worktrees, with sequential rebase-and-merge to keep
linear history.

**Fixed (High):**

- **Multi-program PCR global tracking** (commit `43545ef`, B1, Codex TS-TIME-01).
  Replaced single `last_pcr_27mhz: Option<u64>` field with
  `last_pcr_by_pid: HashMap<u16, u64>` so each program's PCR PID has its own
  time base. Multi-program TS no longer produces false `PcrAnomaly` events.
- **Mux PSI multi-program backpressure** (commit `577abb1`, B2). New
  `Muxer::psi_packets_due` helper centralises reservation math (was hardcoded
  `2` in all 4 push paths; correct shape is `1 + programs.len()`).
  `maybe_emit_psi` now writes `psi_last` for ALL programs on emit.
- **PUSI pointer_field continuation + N-of-M sync re-acquisition**
  (commits `07bbed8` + `b2ceab3`, B3+B7 + followup). PSI assembler now
  honours `pointer_field` continuation bytes (prior section completed first,
  then new section started). Sync re-acquisition uses ffmpeg's 5-of-7
  188-byte-stride validation, no longer false-syncs on isolated `0x47` bytes.
  Mid-stream-join scenario regression-test added in followup.
- **DVB-sub data_identifier strict mode** (commit `ac4db93`, C10,
  Codex 02 #6). Strict mode rejects `data_identifier != 0x20`; lenient
  emits sample + `NonConformantIssue::DvbSubDataIdentifier { observed }`.
- **SRT payload size threading** (commit `a0d7d24`, C1, Codex SRT-01).
  New `Socket::payload_limit() -> usize` returns post-handshake
  `SRTO_PAYLOADSIZE`. `SrtTransport::new` queries it instead of hardcoding
  `1316`. URL `payloadsize=1456` now actually takes effect.
- **OverflowPolicy::Reject surface** (commit `3be1096`, C2, Codex PIPE-01).
  `GapBufferError::Full` now maps to `TransportError::Backpressure("gap buffer full")`
  instead of silent `let _ = gap.enqueue(...)`. Counter-test guards
  `DropOldest` continues silent-evict per its contract.
- **PES header validation + PTS anomaly + subtitle alignment**
  (commit `1cc0653`, B4+B5+B6). `NonConformantIssue::{PtsAnomaly,
  MissingRequiredPts, PesHeaderMalformed, SubtitleAlignmentMissing}`
  variants + `PesHeaderMalformedKind` enum. PTS no longer poisons
  `last_pts_by_pid` when absent. Forbidden `PTS_DTS_flags = 0b01` rejected.
  DVB-sub/teletext PES missing `data_alignment_indicator` surfaces issue.
- **PCR-only adaptation-field injection** (commit `a2445f8`, C3,
  Codex TS-TIME-02). New `Muxer::maybe_emit_pcr_only` injects PCR-only
  TS packets on the PCR PID when no payload arrives within
  `pcr_interval_ms`. Honours H.222.0 Annex D 100ms cap when video/audio
  frame intervals exceed it. CC of PCR-only packets does NOT increment.
- **PesPtsField::PtsAndDts + Annex-B AU validation** (commit `8938ca7`,
  C4+C13). New `Muxer::push_video_to_with_dts(handle, nal, pts, dts, key_frame)`
  API emits `PTS_DTS_flags = 0b11` + correct 5-byte PTS + DTS with marker
  prefixes (`0b0011` PTS, `0b0001` DTS). B-frame reordered video now muxes
  correctly. `validate_annex_b` rewritten as structural NAL walker
  (rejects empty NALs + malformed start-code structure).
- **AC-3 mandatory audio descriptor + syncframe parser** (commit `0ead2f9`,
  C6+C12, Codex AUDIO-01/04). New `codec::ac3` module + `Ac3SyncInfo`
  struct + `parse_syncframe` API per A/52 §5.4.1. Muxer auto-emits
  `ac3_audio_stream_descriptor` (tag 0x81) for `AudioCodec::Ac3`. Demuxer
  emits `NonConformantIssue::Ac3SyncMissing` when
  `data_alignment_indicator=1` but payload doesn't start at `0x0B77`.
- **AAC PCE channel layout + LATM/LOAS sync** (commit `c9835b9`, C7+C11,
  Codex AUDIO-02/03). New `AacChannelLayout::{PceDefined, Channels(u8)}`
  enum — `decode_channels(0)` returns `PceDefined` instead of error.
  Iterator no longer terminates on PCE-prefixed frames. New
  `codec::aac::latm` module validates LOAS syncword
  (`0x2B7` 11-bit pattern) + audioMuxLengthBytes per ISO/IEC 14496-3 §1.7.
- **AV1 binding-conformant mode + AV01 reg first** (commits `5394c00`
  + `78d9b8e` + `2d73294`, C8+C9). New `Av1CarriageMode::{Mpeg2TsBinding,
  InteropRawObu}` enum, default `Mpeg2TsBinding`. Mux emits
  `stream_id=0xBD` + `ts_open_bitstream_unit` framing with spec-correct
  3-byte `[0x00, 0x00, 0x01]` start code + emulation prevention bytes
  (escape rule covers `b ∈ {0x00, 0x01, 0x02, 0x03}` after `0x00 0x00`).
  Demux unwraps binding bytes + surfaces `Av1WrongStreamId` /
  `Av1MissingTsObuFraming` diagnostics. PMT descriptor cache reorders
  caller-supplied AV01 registration descriptor to position 0.

**Fixed (Medium):**

- **AV1 implicit color_range bit** (commit `dd42c33`, B11). `ColorInfo`
  populated unconditionally on well-formed sequence headers — the
  `color_range` wire bit was being read but discarded when
  `color_description_present_flag=0`.
- **NAL/OBU header validation + AV1 uvlc cursor fix** (commit `5d47391`,
  B9+B10). `NonConformantIssue::{NalHeader, Av1ObuHeader}` variants +
  `NalHeaderKind` + `Av1ObuHeaderKind` enums. H.264/265/266 `forbidden_zero_bit`,
  H.265/266 `temporal_id_plus1!=0`, H.266 `reserved_zero_bit` and
  `layer_id ∈ 0..=55` constraints enforced. H.266 `ReservedBit` and
  `LayerIdOutOfRange { id > 55 }` NALs unconditionally dropped per spec
  mandate. AV1 `uvlc()` now consumes the trailing 1-bit marker even on
  the 32-leading-zeros sentinel path.
- **PAT cleanup + PCR field validation** (commit `7752ec8`, B8+B12).
  On PAT change, all per-PID state (cc_by_pid, last_pts_by_pid,
  last_pcr_by_pid, reassembly state, stream_kind_by_pid, pid_to_program,
  PSI assemblers) cleared for removed programs' PIDs. PCR validation:
  reserved 6 bits + extension ≤ 299 per H.222.0 §2.4.3.5.
  `NonConformantIssue::PcrMalformed { kind }` + `PcrMalformedKind` enum.

**Fixed (Medium, breaking — pre-1.0 per `feedback_break_freely_prerelease.md`):**

- **Descriptor builders return Result** (commit `f88f036`, C5,
  Codex 02 #5). `descriptors::{registration, user_private,
  user_private_with_tag, component}` now return
  `Result<Vec<u8>, DescriptorError>` instead of silently truncating via
  `body_len as u8`. `DescriptorError::TooLarge { tag, len, max }`
  variant added. `registration()` body cap corrected 251→255 (additional
  cap 247→251) per spec H.222.0 §2.6. 10 caller sites updated in
  `tst-core`, examples, docs.

**Public API changes (pre-1.0, recorded — see `crates/tst-core/public-api.txt`):**

- New `NonConformantIssue` variants (12 total across the sprint).
- New `tst-core` modules: `codec::ac3` (with `Ac3SyncInfo` + `parse_syncframe`),
  `codec::aac::latm` (with `parse_latm` + `LatmFramingKind`).
- New `tst-core` enums: `AacChannelLayout`, `PesHeaderMalformedKind`,
  `PcrMalformedKind`, `NalHeaderKind`, `Av1ObuHeaderKind`,
  `Av1CarriageMode`, `DescriptorError` (and its `TooLarge` variant).
- New `MuxerConfig::av1_carriage` + `DemuxerConfig::av1_carriage` config
  fields with corresponding builder setters.
- New `Muxer::push_video_to_with_dts` + `MuxSender::send_video_to_with_dts`
  methods for B-frame-reordered video.
- `Socket::payload_limit() -> usize` exposed on `tst-srt`.
- `AdtsFrame.channels: u8` field renamed to
  `AdtsFrame.channel_layout: AacChannelLayout` with `.channels() -> Option<u8>`
  accessor.

**Infrastructure / CI:**

- `#[non_exhaustive]` BASELINE bumped 114 → 131 across the sprint
  (`.github/workflows/ci.yml`). 9 new `#[non_exhaustive]` types contributed:
  `PesHeaderMalformedKind`, `PcrMalformedKind`, `NalHeaderKind`,
  `Av1ObuHeaderKind`, `AacChannelLayout`, `LatmFramingKind`,
  `Av1CarriageMode`, `Ac3SyncInfo`, `DvbSubStripResult`. The remaining
  +8 are comment/rustdoc mentions counted by `rg -c` per
  `feedback_baseline_count_projection_undercount.md`.
- New `.gitignore` entry `/.worktrees/` (commit `145c46b`) enables
  parallel-subagent worktree isolation per
  `feedback_per_subagent_worktree_for_parallel_code_changes.md`.
- C ABI variant codes 21-31 assigned to new `TstNonConformantCode`
  entries; `tstrans.h` regenerated.

**Sprint 2 execution shape:** Phase 1 (5 parallel worktrees) → Phase 2
(4 parallel) → Phase 3 (4 parallel) → 2 follow-up fixes (B3+B7 mid-stream-join
bug + C8+C9 wire-format spec-conformance fix in 2 commits). Per-item
two-stage review (spec compliance + code quality) before merge; 4 items
landed APPROVED_WITH_NOTES with minor polish deferred; 2 items required
implementer-iteration fix cycles (B3+B7 critical bug, C8+C9 wire format).

Closeout memory: `project_validate_1_sprint_2_shipped.md`.
**Sprints 3-5 (Waves D/E/F/G/H/I) remain pending** — see
`docs/validate-1/11-phase-2-plan.md` for the per-wave dispositions.

---

## [Previous-Unreleased] — Validate-1 Phase 2 Sprint 1: Wave A wire-format & UB fixes (docs/validate-1/11-phase-2-plan.md §2.1)

**Eight wire-format / UB / parser-correctness fixes from the Validate-1
Phase 1 audit (20 Claude slices + 8 Codex reports at `docs/validate-1/`).**
Shipped as 8 commits on `main` (`3cd175e..9c29400`) on 2026-05-19/20.

**Fixed (Critical):**

- **DVB teletext PES_packet_length truncation** (commit `dbd0cbb`, Phase 2
  plan §A1, Codex 02 #1). For payloads near the previous cap (65490),
  the writer padded to `N*184` then emitted `(N*184 - 6) as u16` which
  silently wrapped modulo 65536 — conformant demuxers mis-framed the PES
  and the downstream subtitle stream corrupted. New
  `dvb_teletext_total_pes_bytes(payload_len, auto_prepend)` helper
  pre-validates against `u16::MAX`; max payload tightened to 65458
  (auto-prepend) / 65459 (caller-supplied data_identifier). 6 boundary
  tests.

**Fixed (High):**

- **C ABI `slice::from_raw_parts(null, 0)` UB safety** (commit `3cd175e`,
  Phase 2 plan §A3, Codex CABI-01). The pre-existing `(NULL, len > 0)`
  guard missed the `(NULL, 0)` case — Rust requires non-null pointer
  even for zero-length slices. New `tst-c/src/ffi_slice.rs`
  with `pub(crate) ffi_slice(ptr, len, name) -> Result<&[u8], i32>`
  applied to 28 sender-side data-path call sites. 4 contract tests.

- **C event arena lifetime correctness** (commit `e2958be`, Phase 2 plan
  §A2, Codex CABI-02 material omission). Event-payload pointer fields on
  `TstEvent` (audio/subtitle/unknown sample, metadata, NAL/OBU, PMT
  descriptor) referenced the input `DemuxEvent` Vec's storage instead of
  the arena's — dangling after `_recv_event` returned. Extended
  `EventArena` with `payload_buf: Vec<u8>` + two-pass collect-then-
  resolve for multi-payload events. 7 inline tests assert C pointers do
  NOT alias input Vec pointers.

- **Bounded PES reassembly tail bytes** (commit `c6acf84`, Phase 2 plan
  §A4, Codex VIDEO-03). Length-driven completion was taking the whole
  reassembly buffer via `std::mem::take`, including bytes past the
  declared `PES_packet_length`. Replaced with
  `part.buf.drain(..total).collect()`; residual dropped along with
  per-PID state (option B per plan). 2 new regression tests.

- **H.266 `walk_ref_pic_list_struct` AbsDeltaPocSt predicate** (commit
  `568c6cf`, Phase 2 plan §A5, Claude slice 11 H266-V1-H1). Walker used
  an `inter_layer_ref_pic_flag`-shaped predicate falsely attributed to
  §7.4.9; spec per H.266 V4 §7.4.9 eq.(150) +
  ffmpeg `vvc/refs.c:522-526` is
  `!((sps_weighted_pred_flag || sps_weighted_bipred_flag) && i != 0)`.
  Cascaded into 1-bit cursor drift on streams with multi-entry RPS where
  `abs_delta_poc_st == 0` at `i ≥ 1`. Helper signature gains the two
  flags; dead `prev_use_ref_pic_list` tracking removed. RED test:
  `TruncatedRbsp { offset_bits: 240 }` (the exact drift).

- **ST 0601 high-numbered tag narrowing** (commit `3600fd3`, Phase 2 plan
  §A6, Codex 03 #3). `apply_typed_tag` called `lookup(tag as u8)` where
  `tag: u32`; future BER-OID tag 258 narrowed to tag 2 (Precision Time
  Stamp Pack) and overwrote `record.timestamp_us`. Option B fix:
  `u8::try_from(tag)` at call site, matching the ST 0102 precedent at
  `klv/st0102/decode.rs:117-128`. 3 regression tests.

- **H.264 PPS `seq_parameter_set_id` range + PPS→SPS cross-validation**
  (commit `00bd703`, Phase 2 plan §A8, Claude slice 09 H264-RV1). PPS
  parse accepted SPS-ID ∈ [0, 255] vs H.264 V15 §7.4.2.2 mandate
  [0, 31]. Adds `CodecParseError::ReservedValue` for out-of-range.
  `parse_parameter_sets` now drops orphan PPS with `tracing::warn!` if
  the referenced SPS isn't in the map. 4 new tests.

- **IMAPB decode special values + decoded bounds check** (commit
  `9c29400`, Phase 2 plan §A7, Claude slice 01 H2 + H3). Decoder didn't
  implement ST 1201.5 §7.2.2 step 1 special-value detection
  (`0xC8...` +∞, `0xD0...` NaN, `0xE0...` BelowMin, `0xE1...` AboveMax
  silently decoded as garbage normal floats) and didn't bounds-check
  against `[min, max]` (`IMAPB(0,100,3)` wire `0x800000` decoded ~128.0).
  New `pub enum DecodedImapb { Value(f64), PositiveInfinity,
  NegativeInfinity, NaN, BelowMin, AboveMax, ReservedSpecial { raw },
  OutOfRange { decoded } }` with `#[non_exhaustive]`. `value()`
  ergonomic accessor returns `Some(f64)` only for `Value`. Cascade:
  3 KLV consumers + 1 proptest. 9 new tests.

**Workspace updates:**

- BASELINE non_exhaustive in `.github/workflows/ci.yml` bumped 113 → 114
  (one new `#[non_exhaustive]` on `DecodedImapb`).
- `crates/tst-core/public-api.txt` baseline regenerated (additive change
  for `DecodedImapb` enum + variants + impls + `decode_imapb` return
  type).

**Note on remaining Wave A items:** None. Sprint 1 closed all 8 Wave A
items end-to-end. Sprints 2-5 (Waves B-I) cover the remaining ~62
Medium + ~110 Low findings + the empirical interop test suite.

## [Unreleased] — Codex Waves 1-6 re-review fixes (docs/plans/2026-05-19-codex-waves-1-6-rereview-fixes.md)

**Three follow-up fixes from a 2026-05-19 Codex comprehensive re-review of
Waves 1-6** (`docs/refactor-1/_codex-waves-1-6-comprehensive-rereview-report.md`),
performed after plan #92 closed the first round of Codex Wave 6 findings:

**Fixed:**

- **C ABI `TstError::NotAvailable` / `TstError::NotFound` now record fresh
  last-error state before returning.** 17 C ABI sites in `crates/tst-c/src/`
  (12 NotAvailable socket_stats accessors + 5 NotFound per-PID codec_stats
  accessors) returned the negative code via `TstError::Foo as i32` without
  calling `set_last_error()` first, leaving stale message visible to
  `tst_get_last_error()`. Each site now uses `record_not_available(msg)` or
  `record_not_found(msg)` — new `pub(crate)` helpers in
  `crates/tst-c/src/error.rs` paired with the existing `record_shell_error`
  / `record_mux_error` / `record_eos` family. 4 new unit tests prove the
  helpers overwrite prior unrelated last-error state.

- **Single-stream KLV C ABI entry points now document the raw-LS-bytes /
  no-AU-cell-pre-wrap contract.** `tst_muxer_push_klv`,
  `tst_mux_sender_send_klv`, and `tst_managed_mux_sender_send_klv` had
  thin or absent rustdoc, leaving binding authors at risk of pre-wrapping
  the 5-byte `Metadata_AU_cell` header (which the muxer auto-prepends for
  `SynchronousMetadata` streams per ITU-T H.222.0 V9 §2.12.4.2 —
  double-wrapping produces unparseable metadata). New rustdoc on each
  entry mirrors the contract documented in
  `memory/reference_klv_au_cell_caller_responsibility.md`. Regenerated
  `tstrans.h` propagates the new blocks into the MUX SENDER section.

- **User-facing docs refreshed for Waves 2-4 API renames.** `guide-mpegts-mux.md`'s
  `push_video` / `push_klv` signature box updated from raw `i64` PTS to
  `Pts90khz` (Wave 2 typed boundary); its `push_klv` example updated to
  include the now-required `metadata_service_id: u8` 3rd arg.
  `guide-klv.md` updated from `EncodeOptions` to `EncodeConfig` (Wave 2's
  `*Options→*Config` rename). `architecture.md`, `guide-pipeline.md`, and
  `guide-mpegts-demux.md` updated from `pipeline::pairing` to
  `tst_pipeline::ext::pairing` (Wave 4 module move). Plus a bonus stale
  `pts_to_duration(pts_90khz: i64)` signature in `guide-mpegts-demux.md`
  caught and updated to `Pts90khz`. Historical references in
  `deferred-features.md` and the "potential" cross-reference at
  `guide-mpegts-demux.md:409` intentionally left as-is — these describe
  the deferral itself.

**New CI ratchet:**

- `scripts/check-no-direct-not-available-not-found-cast.sh` (the **10th**
  bash ratchet) forbids the `TstError::NotAvailable as i32` /
  `TstError::NotFound as i32` direct-cast pattern in `crates/tst-c/src/`.
  Excludes `crates/tst-c/src/error.rs` (where the helpers' own bodies
  legitimately contain the cast paired with `set_last_error`). Wired
  into `.github/workflows/ci.yml` alongside the existing 9.

**Public API impact:**

- Zero public Rust API delta on all 3 ratcheted crates (`tst-core`,
  `tst-pipeline`, `tst-srt`). New helpers are `pub(crate)`.
- `#[non_exhaustive]` BASELINE in CI: unchanged at 113.
- `tstrans.h` byte delta: +~90 lines from the 3 new KLV docstring blocks
  (cbindgen propagates Rust rustdoc into the header), 0 symbol changes.

**Test coverage:** 4 new unit tests in `crates/tst-c/src/error.rs` covering
the 2 new helpers' code-and-message overwrite behavior. All 10 bash
ratchets green. All 3 cargo-public-api baselines clean.

---

## [Unreleased] — Codex Wave 6 validation fixes (docs/plans/2026-05-19-codex-wave-6-validation-fixes.md)

**Three follow-up fixes to Wave 6 sign-off**, surfaced by a 2026-05-19 Codex static
review of the shipped Wave 5.B + 6.A + 6.D implementations
(`docs/refactor-1/_codex-wave-6-implementation-validation.md`):

**Fixed:**

- **C header section dividers no longer repeat.** Wave 5.B's `add_section_dividers`
  post-process in `crates/tst-c/build.rs` walked cbindgen's name-sorted output
  line-by-line and emitted a divider on every classified-section transition.
  With `cbindgen.toml` `sort_by = "Name"`, alphabetic symbol order interleaved
  domains (`tst_clear_*` → INTROSPECTION, `tst_demux_*` → DEMUX RECEIVER,
  `tst_get_*` → INTROSPECTION again, etc.), producing 16 dividers with 7
  domain sections each appearing twice. Replaced with **chunk-then-group-then-emit**:
  pass 1 buffers each doc-comment + declaration block classified by section;
  pass 2 emits header content verbatim, then iterates 7 required sections +
  2 conditional catch-alls in declared order, emitting each at most once.
  Result: `crates/tst-c/include/tstrans.h` now has 9 dividers (7 required +
  LIFETIME + OTHER), matching the original Wave 5.B spec. Implementation
  required two adaptations beyond the plan's sketch: multi-line declaration
  absorption (cbindgen wraps long parameter lists) and a trailer bucket
  (`} // extern "C"` + `#endif` + `_TST_ABI_ASSERT` block must emit AFTER
  sections, not before). `crates/tst-c/tests/header_drift.rs` carries a
  mirror copy of `add_section_dividers` (intentional — build.rs runs
  pre-compile and cannot import from `tst_c::`); both copies updated in
  lock-step and enforced byte-identical by the existing drift test.

- **`record_mux_error` wildcard for unknown future `MuxSenderErrorKind`
  variants now maps to `TstError::Internal` (was `InvalidConfig`).** Aligns
  with the adjacent `record_shell_error` wildcard at `tst-c/src/error.rs:180`
  (`Internal`) and with `MuxError::kind()`'s wildcard at
  `tst-core/src/error.rs:631` (`Internal`). Rationale: an unknown future
  coarse kind is more truthful surfaced as a library/runtime failure than
  as caller-side `InvalidConfig`. Behavior change is in the future-only
  path — no current variant takes the wildcard.

**Changed:**

- **`mpegts::mux::mod.rs` shrunk from 629 LoC to 320 LoC** (Wave 6.A
  follow-up). `Muxer::new` is now a thin ~50-LoC coordinator; per-program
  state collection, PCR PID resolution, PMT descriptor cache construction,
  and per-stream stats initialization moved to 4 new `pub(super)` helpers
  in `mpegts::mux::state`:
  - `collect_stream_states(prog) -> (Vec<Video>, Vec<Klv>, Vec<Audio>, Vec<Sub>)`
  - `resolve_pcr_pid(prog) -> u16`
  - `build_pmt_descriptor_cache(prog) -> Vec<Vec<u8>>`
  - `initialize_stats(prog, &video, &klv, &audio, &subtitle, &mut into)`

  Final `state.rs`: 445 LoC (was 96). **Zero public Rust API delta**
  (`cargo public-api -p tst-core --simplified` baseline byte-identical).
  **Zero `#[non_exhaustive]` BASELINE delta** (stays 113). Zero behavior
  change — mechanical extraction with all 761 tst-core tests + workspace
  suite green.

**New CI ratchet:**

- `scripts/check-c-header-section-uniqueness.sh` (the **9th** bash ratchet)
  asserts `tstrans.h` has 7-9 dividers AND all section names are unique.
  Guards against regression to the pre-fix line-by-line transition-emission
  shape. Wired into `.github/workflows/ci.yml` alongside the existing 8.

**Public API impact:**

- `cargo public-api` baselines for `tst-core` / `tst-pipeline` / `tst-srt`:
  byte-identical to pre-plan.
- `#[non_exhaustive]` BASELINE in `.github/workflows/ci.yml`: unchanged at 113.
- `tstrans.h` byte delta: cbindgen output reordering (sort-by-name groups
  now travel as section blocks instead of interleaved) + 9 divider lines
  emitted in canonical order instead of 16 in transition order.

**Test coverage:** no new tests added — the byte-identity header drift test
(`crates/tst-c/tests/header_drift.rs`) covers the post-process change end-to-end;
existing muxer roundtrip + descriptor + per-stream-class test suites cover
the `Muxer::new` extraction behavior-equivalence. All 9 bash ratchets green;
all 3 cargo-public-api baselines clean.

---

## [Unreleased] — Wave 6.D `MuxError` two-tier reshape (docs/plans/2026-05-19-wave-6-muxerror-reshape.md)

**Breaking change (tst-core / tst-c — new public surface, C routing simplified):**

- **`MuxSenderErrorKind` enum added** (`tst_core::error::MuxSenderErrorKind`,
  `#[non_exhaustive]`) — 5 coarse categories for the inner (muxer-specific)
  error tier: `InputMalformed`, `ConfigInvalid`, `InvalidUsage`, `Backpressure`,
  `Internal`. Complements the outer `tst_pipeline::ShellErrorKind` (6 variants,
  shell-agnostic) without overlapping it.

- **`MuxError::kind()` method added** — `pub fn kind(&self) -> MuxSenderErrorKind`
  with an exhaustive 32-arm match over every `MuxError` variant, categorizing
  each to its canonical inner-tier kind. Bindings that need coarse routing
  (e.g. "is this a caller bug or a data problem?") can call `kind()` rather than
  matching the full 32-variant set.

- **`mpegts::mux::_detail` module added** — `pub mod _detail { pub use
  crate::error::MuxError; }`. The underscore prefix signals spec-domain tier:
  bindings that need to match individual `MuxError` variants for diagnostic
  output import via `use tst_core::mpegts::mux::_detail::MuxError;`, making
  the non-default, high-specificity import path legible at the use site.

- **`record_mux_error` rewritten** (`tst-c/src/error.rs`, 189 → ~75 LoC) —
  two per-variant overrides kept explicit (`InvalidNal` → `TST_E_INVALID_NAL`,
  `KlvTooLarge` → `TST_E_KLV_TOO_LARGE`); all remaining 30 variants routed via
  `e.kind()` pattern match. Error messages now come from `e.to_string()` (the
  `#[error]` attribute) rather than duplicated per-arm format strings.

- **New CI ratchet** — `scripts/check-mux-error-kind-coverage.sh` verifies
  every `MuxError` variant is matched explicitly in `MuxError::kind()` before
  the `_ => Internal` wildcard arm. Registered in `.github/workflows/ci.yml`
  alongside the existing 3 error-coverage ratchets.

**Public API impact:**

- `cargo public-api -p tst-core --simplified` baseline refreshed: +1 enum
  (`MuxSenderErrorKind`, 5 variants + trait impls), +1 method
  (`MuxError::kind`), +1 module (`mpegts::mux::_detail` with all 32 `MuxError`
  re-exports).
- `cargo public-api -p tst-pipeline --simplified` byte-identical to pre-plan.
- `cargo public-api -p tst-srt --simplified` byte-identical to pre-plan.
- `#[non_exhaustive]` BASELINE in `.github/workflows/ci.yml` bumped **105→111**
  (empirical; `rg -c` counts attribute instances + comment-line mentions).

**Test coverage:** 34 new integration tests in
`crates/tst-core/tests/mux_error_kind_routing.rs` — one assertion per `MuxError`
variant routing plus 2 kind-property tests. All 8 bash ratchets green.

---

## [Unreleased] — Wave 6.B `mpegts/demux/demuxer.rs` god-module split (docs/plans/2026-05-19-wave-6b-demuxer-split.md)

**Refactor (purely internal — zero public API change, zero `#[non_exhaustive]` BASELINE delta):**

- `demuxer.rs` 3584 → ~2312 LoC. The coordinator now contains only the
  public surface methods (`new`, `with_options`, `feed`, `feed_aligned`,
  `next_event`, `flush`, `stats`, `reset_stats`, `stream_codec_stats`), the
  `Demuxer` struct definition, and the thin private dispatch helpers
  (`process_packet`, `handle_process_packet_result`, `lookup_stream`,
  `program_number_for_pid`). All struct fields are `pub(super)`.

- **5 new sibling submodules** extracted from `demuxer.rs`:
  - `sync_ingress.rs` — byte-stream sync recovery, PCR gap tracking,
    continuity-counter validation.
  - `pmt_classify.rs` — PMT stream-type classification and `StreamKind`
    derivation helpers (including `classify_0x06`, `classify_0x06_with_ambiguity`,
    `classify_klv`, `stream_type_from_kind`).
  - `psi_topology.rs` — PSI section dispatch, PAT/PMT topology tracking,
    `build_program_map`, `klv_mismatch_insert`.
  - `pes_emit.rs` — PES reassembly dispatch and complete-PES-to-`DemuxEvent`
    conversion (`handle_pes_packet`, `handle_complete_pes`).
  - `stats_recorder.rs` — stats accounting and nonconformant event queueing
    (`queue_nonconformant`, `bump_video_counters`, `bump_klv_counters`,
    `bump_audio_counters`).

  All sibling submodules are `mod` (not `pub mod`) — private to the `demux`
  tree. Each uses `impl super::demuxer::Demuxer { pub(super) fn ... }` per
  Decision DB3, keeping the coordinator struct in one place.

- Binding-canonical-workflow audit: zero items promoted to `low_level` —
  all extracted helpers are classification/accounting internals with no
  documented FFI or binding-consumer demand.

**No public API impact:**

- `cargo public-api -p tst-core --simplified` byte-identical to pre-plan.
- `cargo public-api -p tst-pipeline --simplified` byte-identical to pre-plan.
- `cargo public-api -p tst-srt --simplified` byte-identical to pre-plan.
- `#[non_exhaustive]` BASELINE in `.github/workflows/ci.yml` stays at **87**.

---

## [Unreleased] — Wave 6.C-KLV typed-set module reorg (docs/plans/2026-05-19-wave-6-klv-reorg.md)

**Refactor (purely internal — zero public API change, zero `#[non_exhaustive]` BASELINE delta):**

- `klv::st0601` fan-out — 1711-line `mod.rs` god-file extracted into:
  `model.rs` (`UasDatalinkLs`, `EncodeConfig`, `GeoPoint`, `Attitude`,
  `FieldOfView`, `Corners`), `decode.rs` (4 decode entry points + inner
  helpers), `encode.rs` (5 encode / len functions). `mod.rs` becomes a
  ~35-line thin facade; all canonical re-exports preserved byte-identically
  at `klv::st0601::*`.
- `klv::st0102` fan-out — 1242-line `mod.rs` extracted into `model.rs`
  (`SecurityLs` + `pub(super)` UTF-16 helpers), `decode.rs`, `encode.rs`.
  `mod.rs` becomes a ~25-line thin facade.
- `klv::st0903` fan-out — 1572-line `mod.rs` + 1012-line `vtarget_pack.rs`
  extracted into `model.rs` (`VmtiLs`), `decode.rs`, `encode.rs`, `tests.rs`,
  and a nested `vtarget_pack/` subdirectory (`mod.rs`, `model.rs`, `decode.rs`,
  `encode.rs`, `tests.rs`). `mod.rs` becomes a ~90-line thin facade.
- `klv::st0605` directory conversion — 219-line `st0605.rs` single-file
  converted to `st0605/{mod.rs, model.rs, decode.rs, encode.rs}` for shape
  uniformity. Tests stay inline in `mod.rs` per Decision K5.
- `## Spec coverage` rustdoc blocks added to all 4 typed-set `mod.rs` files,
  listing parsed tags/fields, `unknown`-preservation policy, decode/encode
  modes, and deferred items. Closes audit `04-documentation.md` Finding 4
  and `08-test-infrastructure.md` Finding 4 (spec-coverage docstring scope).

**No public API impact:**

- `cargo public-api -p tst-core --simplified` baseline refreshed (re-export
  path resolution churn for some impl blocks; zero callable-symbol delta).
- `cargo public-api -p tst-pipeline --simplified` byte-identical to pre-plan.
- `cargo public-api -p tst-srt --simplified` byte-identical to pre-plan.
- `#[non_exhaustive]` BASELINE in `.github/workflows/ci.yml` unchanged by this
  plan (Wave 6.C-codec already bumped 87→105 — that plan's entry was
  inadvertently omitted from CHANGELOG during its ship; covered by memory
  entry `project_plan_87_wave_6_C_codec_reorg_shipped.md`).

**Test coverage:** 761 `tst-core` lib tests pass (unchanged count). All 4
KLV-touching fuzz targets (`klv_iter`, `klv_st0601_decode`, `klv_st0102_decode`,
`klv_st0903_decode`) compile clean under `cargo +nightly fuzz check`. All 6
bash ratchets green.

---

## [Unreleased] — Wave 6.A `mpegts/mux/mod.rs` god-module split (docs/plans/2026-05-19-wave-6-mux-mod-split.md)

**Refactor (purely internal — zero public API change, zero `#[non_exhaustive]` BASELINE delta):**

- `mpegts/mux/mod.rs` broken from ~4300 LoC into 8 focused modules:
  - `mux/state.rs` — stream-state structs (`VideoStreamState`, `KlvStreamState`,
    `AudioStreamState`, `SubtitleStreamState`), `validate_annex_b`,
    `caller_has_recognized_subtitle_descriptor`, `ts_packets_for`.
  - `mux/scheduling.rs` — `psi_due`, `pcr_due`, `maybe_emit_psi` (all `pub(super)`).
  - `mux/stats_accounting.rs` — `MuxerStats` struct and `stats()`, `reset_stats()`,
    `stream_codec_stats()`, `bump_*_counters()`.
  - `mux/push_video.rs` — `Muxer::push_video` and `push_video_to`.
  - `mux/push_klv.rs` — `Muxer::push_klv` and `push_klv_to`.
  - `mux/push_audio.rs` — `Muxer::push_audio`, `push_audio_to`,
    `audio_handles`, `audio_handles_for_program`, `audio_stream_handle`.
  - `mux/push_subtitle.rs` — `Muxer::push_subtitle_to`, `subtitle_handles`,
    `subtitle_handles_for_program`, `subtitle_stream_handle`.
  - `mux/tests/` — 6 test files (`config.rs`, `handles.rs`, `push.rs`,
    `stats.rs`, `subtitle.rs`, `validation.rs`) declared via `#[path]`
    as direct children of `mux` so `use super::*` scope is preserved.
- `mod.rs` reduced from ~4300 LoC to ~590 LoC (coordinator: struct definition,
  `new`, `pull`, `pending_packets`, `capacity_packets`, `pcr_pid_for_program`,
  and module declarations).
- Decision D7 applied: `emit.rs` extraction skipped — the emit loop is
  tightly coupled to per-push adaptation-field state and extracting it
  would require a behavioral-change-risking refactor. Per-push modules
  (`push_video.rs` etc.) own their emit loops directly.

**No public API impact:**

- `cargo public-api -p tst-core --simplified` byte-identical to pre-plan.
- `#[non_exhaustive]` BASELINE in `.github/workflows/ci.yml` unchanged by this
  plan (Plan C-codec already bumped 87→105).
- All 760 `tst-core` lib tests pass without modification.

---

## [Unreleased] — Wave 6.F mechanical / hygiene sweep (docs/plans/2026-05-19-wave-6-mechanical-sweep.md)

**Refactor (purely internal — zero public API change, zero `#[non_exhaustive]` BASELINE delta):**

- Mutex policy sweep — 23 sites. Applies the Wave 4.B hybrid mutex policy
  (plan #79) to every remaining `.lock().unwrap()` production site in
  `tst-pipeline`:
  - **19 sites in `mux_sender.rs`**: 10 fallible-return methods (`send_*`,
    `*_handles_for_program`) → `.lock().map_err(...)?` with site-specific
    diagnostic string mapped to `MuxSenderError::Broken` (via
    `From<TransportError>`) / `MuxError::ProgramNotFound`; 9 infallible-return
    methods (`*_handles`, `stats`, `socket_stats`, `stream_codec_stats`,
    `reset_stats`, `is_alive`) → `if let Ok(...) { ... } else { <safe default> }`
    matching the `socket_stats` precedent (reconnect/mod.rs:419-422).
  - **4 sites in `reconnect/mod.rs`** (`<ManagedTransport as Transport>`'s
    `max_payload`, `is_alive`, `close` + the multi-line site in `send_managed`'s
    pre-check size guard) → safe-default shape via
    `.lock().ok().and_then(...).unwrap_or(...)` for the trait methods, and
    `.lock().map_err(...)?` for the fallible pre-check site.
  - **Zero new BUG: panic sites** — per Plan F Decision F1, every site is
    recoverable (no in-flight queued bytes that would be silently lost on
    lock recovery).
- `apply_query_pair` split — `tst-srt/src/url.rs:343-523`. Decomposes the
  180-line `match`-arm-soup into 22 free-function helpers grouped by
  query-parameter family + a slim ~30-line routing match (24 arms after
  collapsing the latency trio + ffmpeg-alias trio). Per audit
  `01-structure-and-size.md` Finding 8 (Option (b) — "smallest change").
- `#[allow(clippy::field_reassign_with_default)]` tightening — per-site
  evaluation of the 16 sites in `tst-srt/src/config.rs` (4) and
  `tst-core/src/klv/st0601/mod.rs` (12). 13 converted to
  `..Default::default()` struct-update syntax; 3 kept on intentional
  spec-style `UasLs` round-trip construction. Per Plan F Decision F7 +
  audit `07-internal-hygiene.md` Finding 6.
- `#[allow(clippy::unnecessary_cast)]` verification — single site in
  `tst-srt/src/error.rs`. The existing 5-line cross-platform comment was
  re-verified as current; no edit needed. Per audit Finding 7.

**Tests (new regressions, in-file in `crates/tst-pipeline/src/{mux_sender,reconnect/mod}.rs`'s `#[cfg(test)]` mods):**

- `mux_sender_inner_lock_poisoned_returns_broken_error` — covers the 10
  fallible-return mutex sweep sites.
- `mux_sender_inner_lock_poisoned_returns_safe_default` — covers the 9
  infallible-return mutex sweep sites.
- `managed_transport_inner_lock_poisoned_returns_safe_default` — covers
  the 3 `Transport` trait impl sites in `reconnect/mod.rs`.
- Plan #79's `successful_reconnect_does_not_deadlock` regression stays
  passing — Plan F doesn't change `send_managed`'s scoped-guard discipline.

**No public API impact:**

- `cargo public-api -p tst-core --simplified` byte-identical to pre-plan.
- `cargo public-api -p tst-pipeline --simplified` byte-identical to pre-plan.
- `cargo public-api -p tst-srt --simplified` byte-identical to pre-plan.
- `#[non_exhaustive]` BASELINE in `.github/workflows/ci.yml` stays at **87**.

**Lock-policy rustdoc updates:** Both `MuxSender` and `ManagedTransport`
struct-level `# Panics` / "Lock poisoning policy" rustdoc sections updated
to reflect the now-complete hybrid policy across all transport-facing
methods.

**Wave 6 status after Plan F ships:** Plan F is the first of Wave 6's 5
Phase-1 plans (parallel with A, B, C-KLV, C-codec). Plans D and E (Phase 2)
wait on A and B respectively. Once all 7 land, refactor-1 is **complete**
and the project moves to `srt-jni` binding work.

---

## [Unreleased] — Wave 5.C C examples retrofit + tst-c structural reorg + sender-side audio/subtitle C ABI (docs/plans/2026-05-21-c-abi-examples-and-tst-c-reorg.md)

**Added (purely additive — no breaking changes):**

- Sender-side audio + subtitle C ABI exposure (gap left when plans #21
  and #22 deferred their C-side exposure "to the future receiver-surface
  plan," but receiver-surface plans #59/#60/#62 never picked them up).
  New entry points:
  - `TstAudioCodec` enum (Mp2/Aac/AacLatm/Ac3) reused from the
    pre-existing demux-event-side definition.
  - 2 audio constructors: `tst_mux_config_add_audio_stream` +
    `tst_mux_config_add_audio_stream_with_language` (3-byte ISO 639-2
    language tag).
  - 4 per-variant subtitle constructors:
    `tst_mux_config_add_subtitle_stream_dvb_subtitling`,
    `_dvb_teletext`, `_cea708`, `_webvtt`. Per-variant (not tagged
    union) for JNI/UniFFI binding ergonomics.
  - 4 muxer push: `tst_muxer_push_audio[_to]` +
    `tst_muxer_push_subtitle[_to]`.
  - 8 mux_sender send: `tst_mux_sender_send_audio[_to]` +
    `tst_mux_sender_send_subtitle[_to]` plus matching
    `tst_managed_mux_sender_send_*` wrappers (full pattern symmetry
    with the existing video/klv send surface).
  - 15 new integration tests in `crates/tst-c/tests/audio_subtitle.rs`.
- New C example
  `crates/tst-c/examples/c/muxing/mux_with_audio_klv_subtitles.c` —
  first C example covering all four user-visible stream-handle types
  (`TstVideoStreamHandle` + `TstAudioStreamHandle` +
  `TstKlvStreamHandle` + `TstSubtitleStreamHandle`) in one mux program.
  H.264 + AAC-ADTS + ST 0601 KLV + DVB subtitles with synthetic
  payloads; output verified end-to-end with `ffprobe`.

**Improved (zero ABI delta):**

- Retrofitted `crates/tst-c/examples/c/muxing/send_synthetic.c` from
  88 LoC / 19% comment density to 249 LoC / 64% density. Aligned with
  the teaching-code convention bar set by `mux_dual_camera.c` per
  `feedback_examples_are_teaching_code.md`: multi-line header banner,
  WHY comments on every non-obvious API call, explicit error-check
  pattern using `tst_get_last_error_str()`, label-based `goto fail`
  cleanup.

**Internal (zero callable-ABI delta — same symbols, same signatures,
same struct layouts, same sizeof asserts):**

- Split `crates/tst-c/src/config.rs` (1649 LoC) into `config/{mod,
  programs, streams, descriptors, builders}.rs`.
- Split `crates/tst-c/src/demux_receiver.rs` (1054 LoC) into
  `demux_receiver/{mod, events, stats, managed}.rs`.
- Reorganized `crates/tst-c/src/` from 17 flat sibling files into
  `sender/` (5 files: muxer, mux_sender, ts_sender, raw_sender,
  connect) + `receiver/` (4 entries: raw_receiver, ts_receiver,
  demux_receiver/, listen) subfolders. Cross-cutting files preserved
  at the root: `lib.rs`, `error.rs`, `panic.rs`, `handle.rs`,
  `event.rs`, `stats.rs`, `demux_config.rs`, `config/`. Plan A keeps
  version code inline in `lib.rs` (Decision D2); no `version.rs` file
  exists.
- Split `crates/tst-c/tests/url_open.rs` (1421 LoC) into
  `tests/url_open/{mod, mux_sender, ts_sender, raw_sender,
  demux_receiver, ts_receiver, raw_receiver}.rs`. Cargo's
  folder-shaped integration-test discovery (via explicit `[[test]]
  path = "..."` in `Cargo.toml`) treats `url_open/mod.rs` as the
  test binary entry point; `cargo test -p tst-c --test url_open`
  still runs all 31 tests as one binary.
- Added `sort_by = "Name"` to `crates/tst-c/cbindgen.toml` so
  generated items in `tstrans.h` are alphabetically ordered by symbol
  name. Closes a known Plan #83 follow-up. Decouples header layout
  from Rust source-file layout so future reorgs don't churn the
  header. One-time mechanical re-baseline of the committed
  `tstrans.h` (~3184-line diff, all mechanical reordering — same
  callable surface, same struct layouts, same sizeof asserts).
- Added workspace-level `rustfmt.toml` with `reorder_modules = false`.
  Several `tst-c/src/*/mod.rs` files use deliberate non-alphabetical
  `pub mod` declaration order; this config declares the intent so
  rustfmt leaves them alone.

**Verification:**

- All `tst-c` tests pass on default features,
  `--no-default-features`, and `--all-features` (214+ tests, 31 of
  which are the url_open split).
- `cargo public-api` baselines for `tst-core`, `tst-pipeline`,
  `tst-srt` byte-identical (Plan C touches none of those crates'
  Rust public surface; the audio/subtitle work promoted handle
  helpers to `pub #[doc(hidden)]` matching the existing video/klv
  precedent with zero baseline impact).
- `#[non_exhaustive]` BASELINE in `.github/workflows/ci.yml`
  unchanged at 87 (Plan C adds zero `#[non_exhaustive]` decorations).
- All 6 pre-push bash ratchets green (10 new entries added to
  `check-c-abi-rustdoc-coverage.sh` allowlist for the new
  audio/subtitle entry points).
- `cargo clippy --workspace --all-targets -- -D warnings` clean.
- `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
  --all-features` clean.
- `cargo fmt --all -- --check` clean.
- All 3 muxing C examples (`send_synthetic.c`, `mux_dual_camera.c`,
  `mux_with_audio_klv_subtitles.c`) compile cleanly with
  `-Wall -Werror`. Comment density: 64% / 61% / 60%.

---

## [Unreleased] — Wave 5.A C ABI versioning + last-error clear (docs/plans/2026-05-21-c-abi-versioning-and-last-error-clear.md)

**Added (purely additive — no breaking changes):**

- 3-tier C ABI version model: package + ABI + header.
  - **Package version** (tracks `Cargo.toml`):
    - `pub const TST_VERSION_MAJOR/MINOR/PATCH` already existed; emitted
      as `#define TST_VERSION_MAJOR 0` in `tstrans.h` since plan #1.
    - **NEW** runtime accessors: `tst_get_version_major()`,
      `tst_get_version_minor()`, `tst_get_version_patch()`,
      `tst_get_version_packed()` (returns `(M<<16)|(m<<8)|p` matching
      libsrt convention), `tst_get_version_string()` (returns a
      process-lifetime NUL-terminated `"M.m.p"` C string).
  - **ABI contract version** (bumped only on breaking C-ABI change):
    - **NEW** `pub const TST_ABI_VERSION_MAJOR/MINOR = 0/1` (initial
      `0.1` pre-1.0 value). Emitted as `#define TST_ABI_VERSION_MAJOR 0`
      / `#define TST_ABI_VERSION_MINOR 1`.
    - **NEW** runtime accessors `tst_get_abi_version_major()`,
      `tst_get_abi_version_minor()`.
- **NEW** `tst_clear_last_error()` C entry — resets the thread-local
  last-error slot to `(TST_E_SUCCESS, "")`. Mirrors libsrt's
  `srt_clearlasterror()`. Caller-driven; idempotent.
- **NEW** C smoke test
  `crates/tst-c/examples/c/getting-started/version_check.c`.
  Cross-validates every (runtime, header) pair; the canonical pattern
  for binding-author load-time SO/header consistency checks.
- **NEW** Rust integration test `crates/tst-c/tests/version_check.rs`
  (7 tests asserting each runtime accessor returns the expected const).
- **NEW** in-file last-error-clear tests in `crates/tst-c/src/error.rs`
  (`tst_clear_last_error_resets_to_success_state` +
  `tst_clear_last_error_idempotent_when_already_clear`).

**Internal:**

- Decision D1 (see plan): macro prefix is `TST_*` not `TSTRANS_*` for
  consistency with the existing `TST_VERSION_*` / `TST_INVALID_*` /
  `TST_STATS_*` precedent.
- Decision D2: version entries live inline in `crates/tst-c/src/lib.rs`
  rather than a new `version.rs`. Plan C's tst-c reorg owns any
  future extraction.
- Cbindgen mechanism: `pub const FOO: <integer-type> = N;` automatically
  emits as `#define FOO N` (verified by the existing `TST_VERSION_*`
  precedent on HEAD). No `[defines]` config block needed.

**Out of scope (deferred per Wave 5.A scope):**

- ABI-bump CI ratchet (relies on maintainer discipline +
  `header_drift.rs` to catch silent breakage).
- Per-entry-point versioning (`*_added_in` accessors).
- Domain-grouping comments in `tstrans.h` (Plan B).
- Symbol-script restriction of exports (Plan B).
- C ABI `_Static_assert` for `tst_socket_stats_t` (Plan B).

---

## [Unreleased] — Wave 5.B C ABI symbol hygiene + layout asserts + release-validation (docs/plans/2026-05-21-c-abi-symbol-hygiene-and-release-validation.md)

**Tooling / build:**

- **Symbol hygiene.** `crates/tst-c/build.rs` now emits per-OS linker args to restrict
  `libtstrans` dynamic exports to `tst_*`/`TST_*`:
  - Linux: `-Wl,--exclude-libs=ALL` (hides all static-library symbols, including
    libsrt's `srt_*`/`SRT_*` and mbedTLS's `mbedtls_*`).
  - macOS: `-Wl,-exported_symbols_list,exports.txt` (Apple ld whitelist with the
    Mach-O leading-underscore convention).
  - Windows: documented no-op pending plan #65 runtime-test deferral.

  The original plan specified a Linux `-Wl,--version-script=...` mechanism, but
  that conflicts with rustc's auto-emitted anonymous version-script for cdylib
  targets (GNU BFD ld rejects mixing named and anonymous version tags). The
  `--exclude-libs=ALL` pivot achieves the same outcome (0 `srt_*`/`SRT_*` in
  the dynamic export table) without touching the auto-emitted script.

  New file: `crates/tst-c/exports.txt`. Closes audit `09-c-abi.md` Finding 3.

- **Layout assertion.** `crates/tst-c/cbindgen.toml` trailer gains a 9th
  `_TST_ABI_ASSERT(sizeof(tst_socket_stats_t) == 120, ...)` line. Catches
  Rust-side `SocketStats` reorders that change the struct size at C-consumer
  build time. Closes audit `09-c-abi.md` Finding 2.

- **Domain-grouping section dividers.** `crates/tst-c/build.rs` runs a
  post-process step after cbindgen that inserts prefix-keyed section dividers
  in `tstrans.h` (`// ─── INTROSPECTION ───`, `// ─── MUX SENDER ───`, etc.).
  7 required sections (INTROSPECTION, MUX SENDER, TS SENDER, RAW SENDER,
  DEMUX RECEIVER, TS RECEIVER, RAW RECEIVER) + 2 conditional catch-alls
  (LIFETIME, OTHER). Symbol-name-based grouping is independent of source-file
  layout. Closes audit `09-c-abi.md` Finding 5.

- **CI ratchet.** New `scripts/check-no-srt-symbol-leak.sh`; runs
  `nm -D -g --defined-only` against `target/debug/libtstrans.so` + fails on any
  `srt_*`/`SRT_*` match. Wired into `.github/workflows/ci.yml` after the
  existing 6 ratchets. Linux-only (same gate as `symbol_audit.rs`).

- **`crates/tst-c/tests/symbol_audit.rs` update.** Removed the `srt_*`
  allowlist (no longer needed after Task 4); added `srt_symbols_not_exported`
  test for defense-in-depth (clearer failure message naming the specific
  leaked symbol).

**Release-validation:**

- **Step 6 (ffmpeg muxer differential).** `release-validation.sh:138-200` was a
  TODO stub; now extracts H.264 NALs via `ffmpeg -c:v copy`, re-muxes through
  ffmpeg, diffs `tsdump --psi` of `$BASELINE` vs ffmpeg-remux. Soft-fail on diff.
  Skips cleanly if ffmpeg or tsdump are missing.

- **Step 7 (player decode matrix).** `release-validation.sh` Step 7 had a
  partial-stub that only ran `$player --version`; now invokes each of
  `ffplay` / `vlc` / `mpv` / `gst-play-1.0` with player-specific headless flags
  + 10s timeout wrapper; greps stderr for error markers. Soft-fail per Tier-B
  convention. SKIPs missing players.

- **Step 8 (PTS rollover).** New test tool `gen_pts_rollover_fixture` at
  `crates/tst-core/tests/tools/gen_pts_rollover_fixture.rs` emits a synthetic
  .ts file with initial PTS 5 seconds below 2^33; the 10s stream straddles the
  MPEG-TS PTS wraparound. `release-validation.sh` Step 8 invokes the tool +
  probes with `tsdump` to confirm the demux side handles the wrap cleanly.

- **Step 9 (PCR jitter).** New test tool `measure_pcr_jitter` at
  `crates/tst-core/tests/tools/measure_pcr_jitter.rs` walks PCR samples +
  computes inter-PCR delta median + p95 (in milliseconds). Thresholds:
  median > 67 ms or p95 > 100 ms → fail (per
  `reference_ts_corpus_cadence.md` baseline). PCR extraction inlined per
  ISO/IEC 13818-1 §2.4.3.4-5 because `parse_ts_packet` is `pub(super)`.

**Internal (no public-API surface delta):**

- New files: `crates/tst-c/exports.txt`,
  `crates/tst-core/tests/tools/gen_pts_rollover_fixture.rs`,
  `crates/tst-core/tests/tools/measure_pcr_jitter.rs`,
  `scripts/check-no-srt-symbol-leak.sh`.
- Modified: `crates/tst-c/build.rs` (link args + post-process function),
  `crates/tst-c/cbindgen.toml` (9th layout assert),
  `crates/tst-c/include/tstrans.h` (regenerated +1 trailer line + section
  dividers), `crates/tst-c/tests/symbol_audit.rs` (allowlist removal + new
  test), `crates/tst-c/tests/header_drift.rs` (mirrored
  `add_section_dividers` to keep the drift check in sync; kept in lock-step
  with `build.rs` by convention).
- `cargo public-api` baselines for tst-core / tst-pipeline / tst-srt:
  unchanged.
- `#[non_exhaustive]` BASELINE in `.github/workflows/ci.yml`: unchanged.

---

## [Unreleased] — Wave 4.C CancelHandle rename + pairing relocate + polish (docs/plans/2026-05-20-cancelhandle-pairing-and-polish.md)

**Breaking (pre-1.0):**

- Renamed `CancelHandle` → `SrtCancelHandle` to telegraph SRT-specificity
  (the type wraps a libsrt `SRTSOCKET` integer handle with `i64::MIN` as
  the cancelled sentinel — non-SRT transports that arrive later will add
  their own cancel primitives). Re-exports updated at `tst_pipeline::`
  and `tst_srt::` paths. C ABI unchanged (the type is internal to Rust;
  the `tst_*_cancel()` C function family stays the same).
- Relocated `tst_pipeline::pairing` → `tst_pipeline::ext::pairing`. The
  top-level `tst_pipeline::Pairer` (and `PairerMode`, `PairerConfig`,
  `PairerOutput`, `PairerStats`, `VideoSample`, `KlvSample`) re-exports
  are removed — callers spell out `tst_pipeline::ext::pairing::Pairer`.
  Signals "opt-in extension, not first-class shell" by withholding the
  convenience re-export.
- Removed `pub type tst_srt::Result<T> = std::result::Result<T, Error>;`
  alias (zero workspace consumers; workspace standard is to spell out
  the error type).

**Added:**

- Discriminating test files: `crates/tst-core/tests/demux_error_discrimination.rs`
  (3 tests for `DemuxError` variants — `SyncBufExhausted` + strict-mode
  `MalformedPes`/`StrictRejection` + a documented `MalformedPsi` smoke
  fallback since that variant has no public-API trigger path) and
  `crates/tst-pipeline/tests/transport_error_discrimination.rs`
  (5 tests covering `TransportError` variants flowing through shell
  errors: Backpressure, Broken, Closed→EndOfStream on receiver,
  ExplicitClose, TooLarge). Per
  `feedback_audit_test_not_always_discriminating.md`, these assert on
  the specific variant via `matches!` — not on `is_err()`.
- `docs/binding-authors.md` gained a new "Transient vs persistent error
  codes" subsection clarifying the contract on `TST_E_NOT_AVAILABLE` (-13)
  vs `TST_E_NOT_FOUND` (-14) for binding-language retry policies.
- New `tst_pipeline::ext` module with module-level rustdoc codifying
  the opt-in-extension contract for current and future extensions.

**Improved:**

- Upgraded rustdoc on `TstError::NotAvailable` (-13) and `TstError::NotFound`
  (-14) to lead with the transient-vs-persistent contract verb and to
  cross-reference each other. The cbindgen-generated C header
  `crates/tst-c/include/tstrans.h` regenerates with the new rustdoc.
- `tst_pipeline::lib.rs` and `tst_srt::lib.rs` comment blocks for the
  re-exported `SrtCancelHandle` updated from the now-misleading
  "transport-agnostic primitive" framing to the accurate "SRT-shaped
  primitive defined in tst-core for layering reasons" framing.

**Internal:**

- `#[non_exhaustive]` BASELINE in `.github/workflows/ci.yml` unchanged at
  87 (Plan C adds zero `#[non_exhaustive]` decorations).
- Renamed `docs/cancel-handle.md` → `docs/srt-cancel-handle.md` with
  page-header + intro-paragraph rewrites to drop the "universal
  cross-thread shutdown" framing.
- New `scripts/check-raw-c-mapper-coverage.sh` ratchet closes a Wave 1.3
  coverage gap noticed during static closeout review of Wave 4.A. The
  Wave 4.A split of the old `check-tst-c-error-coverage.sh` into
  `check-shell-error-kind-coverage.sh` + `check-pipeline-kind-classification.sh`
  covered the new shell-layer routing but left the raw `record_mux_error`
  / `record_transport_error` mappers (used by standalone-muxer paths and
  by `connect_srt`/`listen_srt` open helpers) unratcheted against
  upstream variant additions. New script restores per-variant coverage
  with one documented exclusion: `TransportError::ExplicitClose`, which
  the raw paths cannot construct.

---

## [Unreleased] — Wave 4.A shell error kind fold (docs/plans/2026-05-20-shell-error-kind-fold.md)

**Breaking (pre-1.0):**

- All 6 pipeline shells now return `struct { kind: ShellErrorKind, source: <Shell>ErrorSource }`
  instead of variant enums (`MuxSenderError`, `SenderError`, `DemuxReceiverError` already
  had enum-shaped errors; `RawSender::send`, `Receiver::next_packet`, `RawReceiver::recv_one`
  now return new shell error types instead of bare `TransportError`). Three new public types
  added: `RawSenderError`, `ReceiverError`, `RawReceiverError`.
- Six new public source enums: `MuxSenderErrorSource`, `SenderErrorSource`,
  `DemuxReceiverErrorSource`, `RawSenderErrorSource`, `ReceiverErrorSource`,
  `RawReceiverErrorSource`. Each `#[non_exhaustive]` with typed `#[from]` variants.
- Callers that matched on `MuxSenderError::Mux(_)` / `SenderError::Transport(_)` / etc.
  switch to `match err.source` or `match err.kind`. Pattern:
  ```rust
  // Old:
  Err(MuxSenderError::Transport(TransportError::Broken(_))) => { /* reconnect */ }
  // New (kind-based — recommended for binding-portable code):
  Err(err) if err.kind == ShellErrorKind::TransportBroken => { /* reconnect */ }
  // OR (source-based — preserves inner-variant discrimination):
  Err(err) if matches!(err.source, MuxSenderErrorSource::Transport(TransportError::Broken(_))) => { /* reconnect */ }
  ```
- New `TransportError::ExplicitClose` variant distinguishes caller-initiated close
  from peer-EOS (the existing `Closed` variant). Runtime wiring lands in Wave 4.B.
- `TsFramingError` is now `#[non_exhaustive]` (workspace convention sweep).
- C ABI numeric TST_E codes unchanged, but the **kind→code mapping consolidates several
  triggers**. The `tst_get_last_error_str()` content preserves the full inner Display
  output so callers reading the string get the full diagnostic:
  - `MuxError::InvalidNal` (was `TST_E_INVALID_NAL = -2`) → `TST_E_INVALID_TS = -3`
  - `MuxError::KlvTooLarge` (was `TST_E_KLV_TOO_LARGE = -5`) → `TST_E_INVALID_TS = -3`
  - `TransportError::TooLarge` (was `TST_E_TOO_LARGE = -6`) → `TST_E_INVALID_TS = -3`
  - `MuxError::InvalidStreamHandle`/`AmbiguousTarget`/etc. (was `TST_E_INVALID_USAGE = -9`) → `TST_E_INVALID_CONFIG = -1`
  - `TransportError::Backpressure` (was `TST_E_TRANSPORT = -8`) → `TST_E_BUFFER_FULL = -4`

**Added:**

- New `tst_pipeline::ShellErrorKind` enum with 6 variants 1:1 with TST_E codes:
  `ConfigInvalid` (-1), `InputMalformed` (-3), `Backpressure` (-4),
  `TransportBroken` (-8), `Closed` (-7), `EndOfStream` (-12). `#[non_exhaustive]`.
- New `tst_pipeline::ShellError` trait: `fn kind(&self) -> ShellErrorKind`. Implemented
  by all 6 shell error types.

**Internal:**

- `crates/tst-c/src/error.rs` collapses from 4 per-variant `record_*_error` functions
  (~270 lines of per-variant translation) to one `record_shell_error<E: ShellError>(e: &E) -> i32`
  helper plus `tst_error_from_kind(kind: ShellErrorKind) -> TstError`. Inline match
  routing at 2 recv-path sites in `crates/tst-c/src/demux_receiver.rs` also
  collapsed to `record_shell_error` (the open-path sites still use
  `record_transport_error` for raw `TransportError` from connect helpers).
- `scripts/check-tst-c-error-coverage.sh` (plan #70, 134 lines) split into two new
  scripts: `check-shell-error-kind-coverage.sh` (kind→code routing in tst-c) and
  `check-pipeline-kind-classification.sh` (inner-variant→kind routing in tst-pipeline).
- `#[non_exhaustive]` BASELINE bumped 72 → 87 in `.github/workflows/ci.yml`.
- The +5 delta over Plan A's projected `72 → 82` baseline comes from
  net new rustdoc-comment-line mentions of `#[non_exhaustive]` (7 comment
  lines added, 2 deleted = +5 net), not from additional public-API attribute
  decorations. The 10 actual attribute additions match Plan A's projection
  (ShellErrorKind + 6 shell error structs + 3 source enums). Discriminating
  attribute-only count today: 67 (`rg -c` inflated count: 87; difference: 20
  comment-line mentions across `crates/`). Comment sites include the
  `kind_from_transport` pattern notes, the `ShellErrorKind` variant rustdoc,
  and shell-error-source struct docs. See memory entry
  `feedback_baseline_count_projection_undercount.md` for the systemic root
  cause; the CI `BASELINE` constant continues to track the inflated `rg -c`
  count for compatibility with the existing guard expression.

---

## [Unreleased] — Wave 4.B Transport semantics + mutex policy (docs/plans/2026-05-20-transport-semantics-and-mutex-policy.md)

**Breaking (pre-1.0):**

- `ManagedRecvTransport::recv_bytes` returns `TransportError::ExplicitClose`
  on caller-initiated paths (entry check + cross-thread cancel signal),
  replacing `TransportError::Closed`. The reconnect-budget-exhausted exit
  continues to return `TransportError::Closed` (peer-EOS-ish). The
  receive-side shell's `kind_from_transport` (Plan A) maps `ExplicitClose`
  → `ShellErrorKind::Closed` → `TST_E_CLOSED` (-7) and `Closed` →
  `ShellErrorKind::EndOfStream` → `TST_E_END_OF_STREAM` (-12), fixing the
  long-standing peer-EOS-vs-caller-close conflation (03-architecture.md
  Finding 5).

**Internal:**

- Mutex poison sweep in `tst-pipeline`: 4 recoverable-path sites now route
  to `TransportError::Broken` with site-specific messages
  (`managed_receive.rs:179`, `reconnect/mod.rs:193/222/287`); 2
  invariant-critical gap-accumulator sites now panic with `BUG: ...`
  prefix caught by `tst-c`'s `ffi_catch` as `TST_E_PANIC_CAUGHT` (-11)
  (`reconnect/mod.rs:214/226`). Plan #45's `.lock().ok()` cancel-path
  precedent extended to all 6 audit-enumerated sites
  (05-error-handling.md Finding 2). The 17 `.lock().unwrap()` sites in
  `mux_sender.rs` and 4 additional sites in `reconnect/mod.rs`
  (size-precheck inside send_managed, plus `Transport::max_payload`,
  `Transport::is_alive`, `Transport::close`) are out of scope for Plan B
  (not in audit enumeration).

- Three booleans (`closed`, `explicit_close`, `cancelled`) now disambiguate
  ManagedRecvTransport state. The original 2-bool design (`closed` +
  `cancelled`) couldn't distinguish caller-close from budget-exhausted
  (both set `closed=true`); added `explicit_close` set only by caller
  paths so the re-entry gate routes correctly.

- New `# Panics` rustdoc on `ManagedTransport::send_managed` and
  `ManagedTransport::drain_gap_if_alive` documenting the `BUG: gap lock
  poisoned` panic contract.

- New `crates/tst-pipeline/tests/poison_policy.rs` with 4 discriminating
  tests covering Tasks 2-4 behaviors.

---

## [Unreleased] — Wave 3.2 naming consistency + Stats typing (docs/plans/2026-05-19-naming-renames-and-stats-typing.md)

**Breaking (pre-1.0):**

- Renamed `ManagedReceiveTransport` → `ManagedRecvTransport` (symmetric
  with `ManagedTransport` and the underlying `RecvTransport` trait).
  C ABI type names (`tst_managed_demux_receiver_t` etc.) unchanged.
- Renamed `RawSenderStats` → `RawSendStats` and `RawReceiverStats` →
  `RawRecvStats` (the one confusable Stats pair in the workspace).
- Renamed C ABI mirror types `TstRawSenderStats` → `TstRawSendStats`,
  `TstRawReceiverStats` → `TstRawRecvStats`. Header typedefs
  `tst_raw_sender_stats_t` → `tst_raw_send_stats_t` and
  `tst_raw_receiver_stats_t` → `tst_raw_recv_stats_t`. Affects
  `tst_raw_*_get_stats` / `tst_managed_raw_*_get_stats` function
  signatures.
- Changed `StreamStats.stream_type: u8` → `StreamStats.stream_type:
  StreamTypeCode`. C ABI `tst_stream_stats_t.stream_type: uint8_t`
  unchanged (Rust→C bridge calls `.as_byte()` at conversion).
- Removed deprecated `AddrError::Ipv6Unsupported` variant (deprecated
  since plan #29 / 2026-05-06 when IPv6 shipped).

**Added:**

- New `tst_core::mpegts::common::StreamTypeCode` enum:
  `Known(StreamType)` for codes this library recognizes,
  `Unknown(u8)` for codes seen in real-world streams outside the
  typed `StreamType` set. `#[non_exhaustive]`. Methods: `from_byte`,
  `as_byte`, `known`.

**Internal:**

- `#[non_exhaustive]` BASELINE bumped 71 → 72 in `.github/workflows/ci.yml`.

---

## [Unreleased] — Wave 2.3 config conventions and symmetry (plan #72)

### Added

- New `docs/conventions.md` codifies workspace-wide policies for
  Config/Options naming, constructor naming, builder-vs-Default, public
  field policy, and where invariants are enforced.
- New `tst_pipeline::ReceiverConfig` + `tst_pipeline::RawReceiverConfig`
  empty `#[non_exhaustive]` structs. Future receive-side knobs can land
  non-breakingly. Mirror the send-side `SenderConfig`/`RawSenderConfig`
  shape.
- New `tst_pipeline::Pairer::new(video_pid, klv_pid) -> Self` primary
  constructor that delegates to `with_options` with default config.
- New `tst_core::error::MuxError::ConfigInvalid { reason: String }`
  variant for richer `validate()` diagnostics that need formatted
  reasons. Maps to `TstError::InvalidConfig` at the C ABI (same code
  as flat-string `MuxError::InvalidConfig`).
- New `tst_core::mpegts::mux::MuxerProgramConfig::new(program_number,
  pmt_pid) -> Self` in-crate constructor for external callers (now
  required because `MuxerProgramConfig` gained `#[non_exhaustive]` and
  has no `Default` impl).

### Changed (BREAKING — pre-1.0)

- `tst_core::mpegts::demux::DemuxerOptions` renamed to
  `tst_core::mpegts::demux::DemuxerConfig`; also gained
  `#[non_exhaustive]`. Construction via struct literal outside the
  crate no longer permitted; use `DemuxerConfig::default()` and assign
  overrides.
- `tst_pipeline::PairerOptions` renamed to `tst_pipeline::PairerConfig`.
- `tst_core::klv::st0601::EncodeOptions` renamed to
  `tst_core::klv::st0601::EncodeConfig`; also gained
  `#[non_exhaustive]`.
- `tst_pipeline::Receiver::new(transport)` → `Receiver::new(transport,
  ReceiverConfig)`. The config is currently empty; pass
  `ReceiverConfig::default()`.
- `tst_pipeline::RawReceiver::new(transport)` → `RawReceiver::new(
  transport, RawReceiverConfig)`. Same.
- `MuxerConfig::validate()` now raises `MuxError::ConfigInvalid { reason }`
  (richer diagnostic) instead of `MuxError::InvalidConfig(static)` for
  `stream_descriptors` length mismatches. Pattern matches on
  `InvalidConfig` no longer catch this specific case.
- `tst_pipeline::SenderConfig`, `tst_pipeline::RawSenderConfig`,
  `tst_srt::SocketConfig`, `tst_srt::ListenerConfig`,
  `tst_core::mpegts::mux::MuxerConfig`, and
  `tst_core::mpegts::mux::MuxerProgramConfig` ALL gained
  `#[non_exhaustive]`. Cross-crate callers using struct literal syntax
  (incl. `Foo { field, ..Default::default() }`) must migrate to
  default-and-assign: `let mut cfg = Foo::default(); cfg.field = ...;`.
  See `docs/conventions.md` § "Public field policy for `*Config`
  structs" for the canonical construction patterns.

### CI

- `#[non_exhaustive]` BASELINE in `.github/workflows/ci.yml` bumped
  from 58 to 71 (+13 observed by the `rg -c` count CI uses; reflects
  4 new annotations from Tasks 2 + 4 — `DemuxerConfig`,
  `EncodeConfig`, `ReceiverConfig`, `RawReceiverConfig` — plus 6 from
  the codex-required policy sweep — `SenderConfig`, `RawSenderConfig`,
  `SocketConfig`, `ListenerConfig`, `MuxerConfig`,
  `MuxerProgramConfig` — plus 3 doc-comment mentions of
  `#[non_exhaustive]` that the regex naturally captures).
- `cargo public-api` baselines refreshed for `tst-core`,
  `tst-pipeline`, AND `tst-srt` (`tst-srt` now changes because Task 9
  sweeps `SocketConfig` + `ListenerConfig`).

---

## [Unreleased] — demux event fixes (plan #69)

### Changed (BREAKING — pre-1.0)

- **`tst_core::mpegts::demux::StreamId` gained `program_number: u16`
  field.** All construction sites must supply it. The `Demuxer`
  populates it from the PMT via an internal `program_number_for_pid()`
  lookup; falls back to sentinel `0` only for pre-PMT contexts where
  no PMT has been seen yet for the PID. Outside-crate construction of
  `StreamId` literals must add the field; pattern matches on
  `StreamId { .. }` are unaffected.
- **`tst_event_sample_t.program_number` (C ABI)** is now populated
  from the actual stream's owning program. Previously hardcoded to
  `0` (TODO from earlier ABI work). Multi-program demux consumers
  that depended on the always-zero behavior must update.
- **`tst_event_metadata_t.program_number` (C ABI)** same as above.

### Added

- **`NonConformantIssue::MalformedPes { pid, reason }` (Rust API)** —
  malformed PES headers now surface as a non-conformant event in
  lenient demux mode instead of propagating as a fatal error. Strict
  demux modes still escalate. Applies symmetrically to both
  `Demuxer::feed` and `Demuxer::feed_aligned` via a shared internal
  handler so the byte-aligned and packet-aligned feed paths report
  identical issue counts on the same input.
- **`tst_event_sample_t.random_access_indicator` (C ABI, `uint8_t`)** —
  exposes the TS adaptation-field RAI bit on video sample events.
  Zero for non-video sample events. Companion to the Rust-side
  `SamplePayload::Video::random_access_indicator` field added in plan
  #68; this entry plumbs it through the C ABI.
- **`tst_event_sample_t.stream_type` (C ABI, `uint8_t`)** — exposes
  the raw PMT `stream_type` byte on sample events with unknown or
  vendor-specific codecs so C-side consumers can inspect / route
  them. Zero for known stream types that map to a typed payload.
- **`tst_event_discontinuity_t.variant_pid` (C ABI, `uint16_t`)** —
  carries the discontinuity-variant-specific PID. Currently used
  only by `PesOversize` (the offending stream's PID); the existing
  `pid` field continues to mean the parent stream PID. Zero for
  variants that don't have a variant-specific PID.
- **`tst_nonconformant_code_t::TST_NCC_MALFORMED_PES = 19` (C ABI)** —
  C-side discriminator for the new `NonConformantIssue::MalformedPes`
  Rust variant; surfaces in `tst_event_nonconformant_t.code`.

### Fixed

- **`Muxer::push_video()` and `Muxer::push_klv()` now route to the
  correct program in multi-program configs** when the lone stream of
  that kind is not in program-index 0. Previously both hardcoded
  `pack(0, 0)`, so a config with a single video stream in
  program-index 1 (or any non-zero program) silently mis-routed the
  pushed AU. Extracted shared `single_video_handle()` /
  `single_klv_handle()` helpers matching the working
  `push_audio()` / `push_subtitle()` pattern.
- **`mpegts::demux::pes::Reassembler::push` previously dropped the
  new PUSI's payload** if the prior buffer hit `MalformedPes` during
  `parse_complete`. Restructured to insert the new `Partial` state
  up-front and defer the prior-parse error, so lenient-mode recovery
  actually emits subsequent `Sample` events from the same PID after a
  malformed PES instead of stalling.

---

## [Unreleased] — codec-specific per-stream stats (plan #68)

### Added

- **`tst_core::stats::StreamCodecStats`** — `#[non_exhaustive]` tagged
  enum carrying per-PID codec-specific counters. Variants (each
  `#[non_exhaustive]`): `Video { nals_or_obus, random_access_aus }`
  (H.264/H.265/H.266 NAL counts or AV1 OBU counts + random-access AU
  count), `Klv { records }` (BER-TLV record count when a PES carries
  multiple records), `Audio { frames }` (MP2 + AAC-ADTS frame counts;
  LATM + AC-3 fall through to `Unknown` — see deferred-features.md),
  `Unknown` (codec not classified or no codec-specific counters
  defined yet).
- **`Muxer::stream_codec_stats(pid)` and
  `Demuxer::stream_codec_stats(pid)`** — `Option<StreamCodecStats>`
  accessors; return `None` when the PID was never observed on this
  handle. Codec kind is determined eagerly from the configured /
  parsed `stream_type`; counters reset alongside the existing
  `stats_per_stream` reset path.
- **Pipeline-level `stream_codec_stats(pid)` on `MuxSender` and
  `DemuxReceiver`** (both plain and managed-transport variants share
  the same method via the `Transport` generic).
- **`tst-c`: 5 new entry points** —
  `tst_muxer_get_stream_codec_stats`,
  `tst_mux_sender_get_stream_codec_stats`,
  `tst_managed_mux_sender_get_stream_codec_stats`,
  `tst_demux_receiver_get_stream_codec_stats`,
  `tst_managed_demux_receiver_get_stream_codec_stats`.
  All take `(handle, pid, *out tst_stream_codec_stats_t)` and return
  the standard `tst_error_t` discriminant.
- **`tst-c`: `TstStreamCodecStats` `repr(C)` tagged-union (24 B)** —
  `kind` discriminator (one of `TST_CODEC_KIND_UNKNOWN`,
  `TST_CODEC_KIND_VIDEO`, `TST_CODEC_KIND_KLV`,
  `TST_CODEC_KIND_AUDIO`) + arm-specific payload structs
  `tst_codec_video_stats_t` / `tst_codec_klv_stats_t` /
  `tst_codec_audio_stats_t`. `_Static_assert` ABI size guards trip
  consumer-side builds on accidental layout drift.
- **`tst-c`: `TST_E_NOT_FOUND = -14` error code** — returned by the
  `_get_stream_codec_stats` family when the PID was never observed on
  this handle. Distinct from `TST_E_NOT_AVAILABLE` (transient
  managed-reconnect mid-flight) so callers can branch on the
  "PID-typo / wrong-PID" vs "wait and retry" distinction.
- **TS adaptation-field `random_access_indicator` (bit 0x40) now
  extracted** and propagated through PES assembly to
  `SamplePayload::Video::random_access_indicator`. Receiver-side
  `random_access_aus` codec counter uses this signal.
- **`tst_core::codec::util::count_nal_units(buf, codec)`** —
  cross-codec helper for counting NAL units
  (H.264/H.265/H.266 Annex-B start-code scan) or OBUs (AV1 LEB128
  walk) inside a single AU buffer. Shared between the muxer-side
  push-time count and the demuxer-side parse-time count.

### Changed

- **BREAKING** — `SamplePayload::Video` gains a new field
  `random_access_indicator: bool`. `SamplePayload` is already
  `#[non_exhaustive]` at the variant level, but the `Video` payload
  struct is not — outside-crate pattern matches on
  `SamplePayload::Video { ... }` need a `..` rest binding to absorb
  the new field; struct construction at test sites must add
  `random_access_indicator: false` (or the relevant value).
- **BREAKING** — `mpegts::demux::pes::PesPayload` gains a new field
  `random_access_indicator: bool`.
- **BREAKING** — `mpegts::demux::ts::TsPacket` gains a new field
  `random_access_indicator: bool` (struct already `#[non_exhaustive]`
  so pattern-match consumers absorb via `..`; struct-construction
  consumers must add the field).
- **BREAKING** — `mpegts::demux::pes::Reassembler::push` gains a 4th
  parameter `random_access_indicator: bool` (RAI gets latched on the
  first TS packet of an AU at the reassembler level so PES payload
  emission carries it through).
- **`#[non_exhaustive]` workspace count guard `BASELINE`** bumped
  from 52 to 54 in `.github/workflows/ci.yml` (absorbs
  `StreamCodecStats` enum + 3 variants worth of `#[non_exhaustive]`
  attributes).

See the plan at `docs/plans/2026-05-16-codec-specific-stats.md`.
Closes the P1 "codec-specific stats on `StreamStats`" backlog entry
(deferred from plan #16).

---

## [Unreleased] — tst-srt Windows MSVC port (plan #65)

### Changed
- **`tst-srt`: internal sockaddr handling switched from
  `libc::sockaddr_*` to `os_socketaddr::OsSocketAddr`.** Public API
  surface unchanged — `Socket::connect` / `Socket::peer_addr` /
  `Socket::local_addr` / `Listener::bind` / `Listener::accept` all
  still take or return `std::net::SocketAddr`. Internal substitution
  unblocks `*-pc-windows-msvc` builds (`libc` doesn't expose
  `sockaddr_storage` / `sockaddr_in` / `sockaddr_in6` / `linger` /
  `AF_INET6` on that target). `Socket::set_linger`-related code uses
  a hand-rolled `#[repr(C)] LingerOpt` POD (2-field struct, same
  layout on every platform; POSIX `SO_LINGER` predates the BSD /
  Win32 split). `addr::to_sockaddr` is now infallible (was
  `Result<_, AddrError>` with a Result that was vestigial); callers
  in `socket.rs` / `listener.rs` simplified accordingly.

### Added
- **`os_socketaddr = "0.2"` workspace dep** for cross-platform
  sockaddr abstraction. Used only by `tst-srt` today. Tiny crate
  (~400 LoC); deps are `libc` on Unix + `winapi` cfg-gated on
  Windows.
- **`crates/srt-sys/build.rs`: Windows MSVC build support.**
  Three additions cfg-gated on `target.contains("msvc")`:
  - `/EHsc` cxxflag for the libsrt cmake build (MSVC requires
    explicit C++ exception unwind semantics; gcc/clang have it on
    by default). Originally shipped under plan #64 (commit
    `dcd04d6`); referenced here for completeness.
  - Link `srt_static` instead of `srt` (libsrt's CMakeLists names
    the static lib `srt_static.lib` on MSVC to avoid colliding with
    the shared-lib import lib also called `srt.lib`; see
    `vendor/srt/CMakeLists.txt:1169-1181`).
  - Link `bcrypt` (mbedTLS on Windows uses `BCryptGenRandom` from
    `bcrypt.dll` for entropy collection; on Linux it uses
    `/dev/urandom`).
- **`docs/deferred-features.md`: "Windows MSVC runtime test
  stabilization" entry.** SRT loopback tests hang on Windows (at
  least `tst-c::demux_receiver_loopback` observed at 18+ min before
  cancellation; likely the whole loopback test family affected).
  Most plausible root cause: `srt_close` peer-EOS propagation
  semantics differ on winsock vs BSD sockets. Diagnosis requires
  Windows hardware on hand to iterate. Memory note with full
  diagnostic plan at
  `project_plan_65_windows_runtime_test_deferral.md`.

### Skipped on windows-msvc (pending deferred follow-up)
- **`cargo test --doc`, `cargo test` (default / no-default /
  all-features) gated on `if: matrix.name != 'windows-msvc'`**
  in `.github/workflows/ci.yml`. Windows MSVC matrix entry now
  runs `cargo build` (default + no-default-features) to gate
  compile + link regressions; runtime test coverage falls to
  Linux x86_64 + Linux aarch64 + macOS arm64.

### Fixed
- **`tst-srt/tests/socket_stats::socket_stats_returns_none_after_close`:
  50ms pause before close to win the accept/close race.** Same
  fast-hardware race fixed for `lifecycle.rs::explicit_close_succeeds`
  in plan #66 (commit `40eb7f9`); surfaced on linux-aarch64 mid-
  plan-#65 once Windows compile errors stopped masking other
  matrix-entry failures. Order-swap fix from plan #66 doesn't
  apply here because the accept closure calls `recv()` (blocks
  until peer-close), so `accept.join()` before close would
  deadlock; 50 ms pause covers the connect/accept window instead.

### Allow
- **`#[allow(clippy::unnecessary_cast)]` on the
  `crates/tst-srt/src/error.rs` tests module.** bindgen emits the
  `SRT_REJECT_REASON_*` and `SRT_REJX_*` constants as `u32` on
  Linux but `i32` on `*-pc-windows-msvc`. The `as i32` casts in
  the reject-reason ordinal-pinning tests are necessary on Linux
  (u32→i32) but redundant on Windows (i32→i32 → clippy error
  under `-D warnings`). Module-level allow with explanatory
  comment rather than 25+ per-callsite cfg gates.

---

## [Unreleased] — macOS arm64 phase-in stabilization (plan #66)

### Changed
- **Loopback integration tests stabilized for Darwin scheduling.**
  Six tests across `crates/tst-c/tests/` and `crates/tst-srt/tests/`
  had hardcoded `thread::sleep` drain pauses (100-500 ms) that worked
  on Linux loopback but raced on the GHA `macos-14` (Apple Silicon)
  runner. All bumped to 1 s — comfortably covers SRT's 120 ms
  latency budget plus Darwin scheduling jitter on every platform.
  Affected tests: `tst-c::raw_receiver_loopback`,
  `tst-c::ts_receiver_loopback`, `tst-c::stats`,
  `tst-srt::pipeline_sender`, `tst-srt::pipeline_receiver_live`,
  `tst-srt::pipeline_receiver_live_corpus`. (Continues the pattern
  established by the post-plan-#64 hotfix to
  `tst-c::demux_receiver_loopback`.)
- **`crates/tst-c/tests/smoke.rs`: cross-platform cdylib name +
  dylib-search env var.** Was hardcoding `libtstrans.so` +
  `LD_LIBRARY_PATH`; macOS uses `.dylib` + `DYLD_LIBRARY_PATH`,
  Windows uses `tstrans.dll` + `PATH`. Refactored to use
  `std::env::consts::DLL_{PREFIX,SUFFIX}` for the name + a compile-
  time `cfg!`-evaluated const for the env var name. Windows PATH
  handling prepends (not replaces) so basic C runtime DLLs stay
  reachable.
- **`crates/tst-srt/tests/lifecycle::explicit_close_succeeds`:
  deterministic accept/close ordering.** Latent race surfaced by
  plan #67's linux-aarch64 gating promotion — on fast ARM hardware
  `socket.close()` could win against listener `accept()` returning,
  leaving `accept` to panic with "Connection was broken." Swapped
  order: `accept.join()` first, then `socket.close()`. Same
  verification intent; no race.

### Cfg-gated
- **`crates/tst-c/tests/symbol_audit`: `#[cfg_attr(not(all(target_os =
  "linux", target_env = "gnu")), ignore = "..."]`.** The test uses
  GNU nm with ELF-specific flags and filters ELF housekeeping
  symbols (`_init`, `_fini`, `__bss_start`, etc.). macOS (Mach-O)
  and Windows (COFF) have entirely different symbol formats. Linux
  GNU coverage (x86_64 + aarch64, both gating) is sufficient for the
  no-Rust-symbol-leak invariant; porting the test would require
  three separate platform-specific implementations of the same
  invariant. Documented as Linux-GNU-only by design in the module
  rustdoc.

---

## [Unreleased] — Linux aarch64 promoted to gating (plan #67)

### Changed
- **`.github/workflows/ci.yml`: linux-aarch64 flipped from
  `continue-on-error: true` to `continue: false`.** Aarch64 was
  green on every post-ship run since the plan #64 matrix expansion
  (2026-05-16), so the conservative 14-day phase-in window is no
  longer warranted. Aarch64 build/test failure now blocks PR merge
  alongside Linux x86_64.
- **`docs/compatibility.md`: Linux aarch64 row** updated from
  "Tier 1, phase-in" to "Tier 1, gating". macos-arm64 and
  windows-msvc remain "Tier 1, phase-in" pending their own fix
  plans (#66 macOS loopback stabilization, #65 tst-srt Windows
  port).

---

## [Unreleased] — tst-c Tier 1 multi-platform (plan #64)

### Added

- **CI: Tier 1 multi-platform matrix.** `.github/workflows/ci.yml`
  refactored from a single `test-linux` job into a matrix-strategy
  `build` job with 4 entries: Linux x86_64 (gating, unchanged),
  Linux aarch64 (phase-in informational), macOS arm64 (phase-in
  informational), Windows x86_64 MSVC (phase-in informational).
  Native runners on all 4 — no cross-compilation. After ~14
  consecutive green nightly days on the 3 new platforms a separate
  follow-up plan (P2) flips `continue-on-error: true` to `false`,
  converting them to gating.
- **`docs/compatibility.md` build-targets table** documenting Tier 1
  (Linux x86_64 / Linux aarch64 / macOS arm64 / Windows MSVC) +
  Tier 2 (Linux musl) + Deferred (iOS, Android, MinGW, macOS Intel)
  status per platform, with phase-in semantics explained inline.
- **`docs/deferred-features.md` entries** for iOS (device + simulator),
  Android (arm64 + x86_64 emulator + armv7), macOS x86_64 Intel, and
  Windows MinGW — each with concrete consumer-driven triggers and a
  scope-when-added note. iOS + Android gated on the future
  `srt-uniffi` plan; macOS Intel + MinGW gated on specific consumer
  asks.
- **`README.md` Platform support subsection** under `## Building`
  listing the 4 Tier 1 platforms and cross-linking
  `docs/compatibility.md` + `docs/deferred-features.md`. Stale
  "multi-platform builds … next on the roadmap" sentence near the
  C example removed (multi-platform ships today; only `srt-jni` /
  `srt-uniffi` remain on the roadmap).

---

## [Unreleased] — libsrt wire-stats at the C ABI (plan #63)

### Added

- **`tst_core::transport::SocketStats`** — abstract wire-level transport
  stats (RTT µs, send/recv/link bandwidth bps, sent/received bytes +
  packets, recv-side byte+packet loss, send-side packet loss,
  retransmits, send/recv drops, send/recv buffer depths in packets).
  16-field `#[non_exhaustive]` struct so growing the field set in
  future libsrt releases is not a breaking change.
- **`Transport::socket_stats()` / `RecvTransport::socket_stats()`** —
  new trait method, defaulted to `None`. `SrtTransport` /
  `SrtRecvTransport` implement it by mapping
  `crate::socket::Stats` (libsrt `CBytePerfMon` snapshot) through a
  `map_stats` free function. `ManagedTransport` /
  `ManagedRecvTransport` forward through
  `inner.as_ref().and_then(...)`, returning `None` mid-reconnect.
- **`MuxSender::socket_stats()`, `Sender::socket_stats()`,
  `Receiver::socket_stats()`, `RawReceiver::socket_stats()`,
  `DemuxReceiver::socket_stats()`** — pipeline-shell pass-throughs.
  `RawSender` reaches through the existing `transport()` accessor.
- **`tst-c`: 12 new entry points `tst_*_get_socket_stats(p, out)`**
  across all 6 sender + 6 receiver handle families (mux_sender,
  ts_sender, raw_sender, receiver, demux_receiver, raw_receiver —
  each plain + managed). Reads from the underlying libsrt socket and
  copies the snapshot into the caller's `tst_socket_stats_t`.
- **`tst-c`: `TstSocketStats` `repr(C)` struct (120 B)** — 16 fields
  (3 u32 + 1 u32 pad + 13 u64). Const-assert pins size at 120 B.
  Layout documented field-by-field for binding authors.
- **`tst-c`: `TST_E_NOT_AVAILABLE = -13` error code** — returned by
  the `_get_socket_stats` family when the inner transport has no
  live socket (closed, or for managed: mid-reconnect). Distinguished
  from `TST_E_INVALID_USAGE` so callers can branch on the transient
  vs. fundamental distinction.
- **C teaching example `examples/c/operations/socket_stats_poll.c`**
  — 5-second send loop with periodic socket-stats print every 500 ms
  (RTT, bytes_sent, packets_sent, loss, retransmits, send-buffer
  depth). First entry under the new C-side `operations/` subfolder.

### Changed

- **`#[non_exhaustive]` workspace count guard `BASELINE`** bumped from
  42 to 47 in `.github/workflows/ci.yml` (absorbs `SocketStats` +
  4 prior post-plan-#62 additions).
- **`cargo public-api` baselines** refreshed for `tst-core`,
  `tst-pipeline`, `tst-srt` (additions: `SocketStats` struct +
  `socket_stats()` methods on the trait + 6 shells + 2 transport
  impls + `Box<T>` blanket forwarding).
- **Mid-flight catch: `#[non_exhaustive]` outside-crate construction**
  — Rust E0639 blocks `SocketStats { ... }` struct-literal even with
  the `..Default::default()` update-syntax tail (no escape hatch as
  of Rust 1.85). The `map_stats` site in `tst-srt` uses the
  default-and-assign pattern instead.

See the wire-stats plan at
`docs/plans/2026-05-16-tst-c-libsrt-wire-stats.md`.

---

## [Unreleased] — Phase 3 of tst-c receiver surface (plan #62)

### Added

- **`tst-c` receiver surface Phase 3** — `tst_demux_receiver_t` and
  `tst_managed_demux_receiver_t` opaque handles wrapping
  `tst_pipeline::DemuxReceiver<SrtTransport>`. Surface the full typed-
  event API to non-Rust consumers: `tst_event_t` tagged union over
  PROGRAM_MAP / SAMPLE / METADATA / DISCONTINUITY / NONCONFORMANT,
  with subordinate `tst_nal_t`, `tst_obu_t`, `tst_descriptor_t`,
  `tst_stream_info_t`, `tst_klv_link_t` list elements. Pointer fields
  borrow from a per-handle EventArena (zero-alloc steady state) —
  valid until the next `_recv_event` / `_close` call.
- **`tst_demux_config_t` opaque builder** — caller-side knobs:
  strict mode (4 levels), KLV PID→video PID link overrides,
  per-PID stream-kind overrides, PES reassembly caps.
- **Bundled send-side descriptor API** — `tst_mux_config_add_video_descriptor`,
  `_add_klv_descriptor`, `_add_audio_descriptor`,
  `_add_subtitle_descriptor` close the previously-deferred
  per-stream PMT descriptor construction at the C ABI. Shares the
  receive-side `tst_descriptor_t` struct from day one.
- **Per-PID stats** — `tst_demux_receiver_get_stream_stats` returns
  a borrowed `(*const tst_stream_stats_t, size_t)` array per design §4.5
  lifetime convention (valid until next get_stream_stats /
  reset_stats / close call).
- **Two new C examples** — `recv_demux_to_console.c` (flagship
  Phase 3 example printing all 5 event kinds) and
  `recv_klv_to_stdout.c` (KLV byte-flow tap, building block for
  external typed-ST 0601 decoders).
- **`_Static_assert` ABI size guards** on all public Phase 3 structs
  (`tst_nal_t` 24 B, `tst_obu_t` 24 B, `tst_descriptor_t` 24 B,
  `tst_stream_info_t` 40 B, `tst_klv_link_t` 8 B,
  `tst_demux_receiver_stats_t` 48 B, `tst_event_t` ≤256 B) — trip
  consumer-side builds on accidental layout drift.

See the Phase 3 plan at `docs/plans/2026-05-16-tst-c-demux-receiver.md`
and the design doc at `docs/specs/2026-05-15-tst-c-receiver-surface-design.md`.
This ships the complete tst-c receiver surface (Phases 1, 2, 3 all
shipped); next-up is `srt-jni` / `srt-uniffi` cross-language bindings.

---

## [Unreleased] — Phase 2 of tst-c receiver surface

### Added

- **`tst-c` receiver surface Phase 2** — `tst_receiver_t` and
  `tst_managed_receiver_t` opaque handles wrapping
  `tst_pipeline::Receiver<SrtTransport>`. 14 new C entry points
  (open / open_listener / recv_packet / cancel / close / get_stats /
  reset_stats × plain + managed). `tst_receiver_recv_packet`
  delivers one 188-byte aligned MPEG-TS packet per call with sync
  recovery already done; the syncer counters
  (`bytes_skipped_for_sync`, `resync_events`) reach the C consumer
  via the new `tst_receiver_stats_t` struct.
  See `examples/c/receiving/recv_ts_to_file.c` for the teaching
  example. Phase 3 (`tst_demux_receiver_t` + typed events) remains
  on the P0 backlog.

---

## [Unreleased] — Phase 1 of tst-c receiver surface

### Added

- New `TstError::EndOfStream = -12` error code distinguishing peer
  graceful disconnect (FIN) from caller-side `Closed = -7` cancel/close.
- New `tst_raw_receiver_t` opaque handle with the following C entry
  points: `tst_raw_receiver_open(url)`,
  `tst_raw_receiver_open_listener(url)`, `tst_raw_receiver_recv`,
  `tst_raw_receiver_cancel`, `tst_raw_receiver_close`,
  `tst_raw_receiver_get_stats`, `tst_raw_receiver_reset_stats`.
- New `tst_managed_raw_receiver_t` opaque handle with the same 7 entry
  points (managed sibling).
- New `TstRawRecvStats` `repr(C)` struct mirroring
  `tst_pipeline::RawRecvStats`.
- New `tst_*_cancel` entry points for all six sender families
  (`tst_raw_sender_cancel`, `tst_managed_raw_sender_cancel`,
  `tst_sender_cancel`, `tst_managed_sender_cancel`,
  `tst_mux_sender_cancel`, `tst_managed_mux_sender_cancel`) — closes
  the P1 sender-side cancellation deferral.
- New `Mode { Caller, Listener }` enum + `SrtUrl::mode` field on
  `tst-srt`; URL parser now accepts `?mode=listener` and allows empty
  host in listener mode. `tst_*_open_listener` C entry points also
  accept `srt://:port` (empty host) without requiring the explicit
  `?mode=listener` query parameter.
- New C example `recv_raw_to_file.c` (`crates/tst-c/examples/c/receiving/`).

### Fixed

- `tst_raw_receiver_recv` now maps `TransportError::Broken` on a
  non-cancelled handle to `TST_E_END_OF_STREAM` (was incorrectly
  surfacing as `TST_E_TRANSPORT`). SRT peer disconnect surfaces as
  `Broken` at the transport layer to support managed-reconnect; the
  plain C ABI semantically translates this to "end of stream".

### Internal

- Sender handle structs (`TstRawSender`, `TstManagedRawSender`,
  `TstSender`, `TstManagedSender`, `TstMuxSender`, `TstManagedMuxSender`)
  gain a side-channel `Arc<dyn TransportCancel>` + `Arc<AtomicBool>`
  field captured at `_open` time to support thread-safe `_cancel`
  without acquiring the handle's `Mutex`.
- C ABI rustdoc coverage allowlist extended for the 19 new entry
  points; proper `# C ABI` rustdoc backfill on corresponding Rust
  methods deferred to a P2 follow-up.

---

## Unreleased

Phase 1 (SemVer ratchet), Phase 2 (DX + observability), Phase 3
(FFI-readiness), Phase 4 (performance hot paths), Phase 5
(internal hygiene), and Phase 6 (test infrastructure) of the Rust
quality + DX + FFI refactor. Plan #39 (examples reorganization),
plan #44 (KLV wire-format critical fixes from the 2026-05-10
spec-validation audit), plan #45 (pipeline close-flush and pairer
PTS saturation fixes from the same audit), plan #46 (KLV
follow-up: VMTI checksum ordering + Security LS UL constant),
plan #47 (MPEG-TS PSI multi-section reject + AV1 binding docs),
plan #48 (video codec parser robustness fixes), plan #49
(SRT RejectReason mapping fix), and plan #50 (tst-c FFI panic
isolation) also ride this release.

### Added

- **OSS-Fuzz onboarding artifacts** (`oss-fuzz/`): `project.yaml`, `Dockerfile`,
  `build.sh`, and `README.md` configure continuous Google-compute fuzzing for
  the 16 cargo-fuzz harnesses (15 in `tst-core`, 1 in `tst-srt`). Includes
  a shared `klv.dict` libFuzzer dictionary, per-target `.options` files for
  the 4 demux/parser targets, and seed corpora for 14 of 16 targets sourced
  from existing fixtures + committed synthetic seeds. The PR to
  `google/oss-fuzz` is a separate manual step documented in
  `oss-fuzz/README.md`.

### Fixed

- **`parse_pat` / `parse_pmt` OOB on short section_length** — surfaced by
  OSS-Fuzz local smoke (plan #53). Both parsers now reject `section_length`
  below the structural minimum (9 for PAT, 13 for PMT) with the new
  `PsiParseError::SectionTooShort` variant, instead of underflowing the
  CRC slice extraction.

- H.265 SPS parser: fixed bit-cursor misalignment in
  `walk_short_term_ref_pic_sets` that caused `parse_sps` to return
  `ReservedValue { field: "delta_idx_minus1" }` on valid Main10
  conformance vectors. The inter-prediction arm was unconditionally
  reading `delta_idx_minus1` (ue), but per H.265 §7.3.7 that field is
  only signaled when `stRpsIdx == num_short_term_ref_pic_sets` — true
  only in slice-header context, never in SPS context. The bug was
  surfaced by the `DBLK_A_MAIN10_VIXS_4` fixture (plan #55); its entry
  is now removed from the test runner's `KNOWN_PARSER_BUGS` allow-list.

### Testing

- `scripts/release-validation.sh` steps 3-5 (`tsanalyze` / `tspsi` / `ffprobe`)
  now diff against committed golden files at `tests/golden/baseline-*.{txt,json}`
  instead of printing "no golden yet". A new `--update-goldens` flag refreshes
  the goldens in place when a behavior change is intentional. The script exits
  1 on unexpected divergence. Goldens are produced from a baseline generated by
  `cargo run -p tst-examples --example mux_to_file -- baseline.ts 5`.

- New maintainer tool `corpus_to_fixture` at
  `crates/tst-core/tests/tools/corpus_to_fixture.rs` extracts minimal
  TS-packet sub-sequences from corpus `.ts` files (filtered by PID and/or
  packet-index range) into committed regression fixtures at
  `crates/tst-core/tests/fixtures/regression/`. Optional `--emit-shim`
  generates a Cargo integration test that `include_bytes!`s the fixture
  and smoke-tests it through `Demuxer`. Modeled after TSDuck's
  `ts2headers.sh` capture-then-commit pattern. Invoke via
  `cargo run -p tst-core --bin corpus_to_fixture -- --help`.

### Internal

- Test infrastructure: new `common::Loopback` + `AcceptHandle<R>` helper
  in `crates/tst-srt/tests/common/mod.rs` consolidates the 15-line
  "bind / spawn accept / signal ready" boilerplate into a 3-line
  builder + closure shape. 18 of 20 integration tests now use the
  helper (net −90 lines across the sweep). Two files don't fit:
  `ipv6_loopback.rs` (helper hardcodes `127.0.0.1:0`) and
  `listener_accept_timeout.rs` (tests `accept_timeout` itself; spawns
  a connector thread — inverse pattern). Pattern from GStreamer's
  `tests/check/elements/srt.c`. Survey item #5.
- CI: new nightly `sanitizers` workflow (`.github/workflows/sanitizers.yml`)
  runs `cargo test -p tst-core -p tst-pipeline` under AddressSanitizer
  and ThreadSanitizer (separate jobs; sanitizers can't combine). Trigger:
  `schedule: '0 3 * * *'` + `workflow_dispatch`. Scope intentionally
  pure-Rust; libsrt + mbedTLS instrumentation is deferred to a follow-up
  plan that threads sanitizer flags into the vendored cmake build.
  Suppression files at `.sanitizer-suppressions/{asan,tsan}.txt`.
  `continue-on-error: true` for the first 2-3 weeks; tighten once stable.
  Survey item #8.

---

### tst-c FFI panic isolation (2026-05-11) — plan #50

CABI-04 from the 2026-05-10 audit
(`docs/analysis/2026-05-10-audit-slices/15-tst-c-abi.md`). Phase 0
(plan #36, 2026-05-08) wrapped the data path via
`Handle::with_inner_{mut,ref}`; this plan completes the coverage
for the open path, the config-builder setters, and the last-error
accessors. Rust's `extern "C"` panic behavior is implementation-defined
under `panic="unwind"` and aborts under `panic="abort"`; either is
unacceptable for a stable C ABI. After this fix, every panic
inside `tst-c`'s extern "C" boundaries is caught, recorded as
`TstError::PanicCaught` (-11) in the thread-local last-error, and
translated to a sentinel return for the entry point's return type.

#### Fixed (panic-safety hardening)

- **New `crates/tst-c/src/panic.rs` module** with `pub(crate) fn
  ffi_catch<R, F>(default: R, f: F) -> R` helper. Wraps
  `catch_unwind(AssertUnwindSafe(f))`; on `Err` records `PanicCaught`
  via the existing `record_panic_caught` in `error.rs` (extracts a
  best-effort detail string from the panic payload) and returns the
  caller-supplied default sentinel.

- **All 7 `_open` entry points wrapped**: `tst_muxer_open`,
  `tst_mux_sender_open`, `tst_managed_mux_sender_open`,
  `tst_sender_open`, `tst_managed_sender_open`, `tst_raw_sender_open`,
  `tst_managed_raw_sender_open`. Previously bare against panics in
  `Socket::connect_with` / `MuxerConfig::validate` / `MuxSender::new` /
  URL parsing / `Box::new` allocation. Reconnect-time panics on
  managed senders are already covered by `Handle::with_inner_*`
  wrapping (the factory runs from the data path).

- **All 25 config-builder entry points in `config.rs` wrapped**:
  4 `_new` constructors (default `null_mut`), 4 `_free` destructors
  (default `()`), `tst_mux_config_add_program` (default
  `TST_INVALID_PROGRAM_HANDLE`), `tst_mux_config_add_video_stream` /
  `_add_klv_stream` (default `TST_INVALID_STREAM_HANDLE`), and 14
  `c_int` setters covering PCR/PSI/buffer + 3 descriptor setters +
  2 sender setters + 5 reconnect setters (default `TstError::Internal
  as i32`). Vec::push, Vec::with_capacity, parse_tlv_list slice
  arithmetic, and Box::new are all panic surfaces; previously a panic
  in any of them would unwind through extern "C".

- **Both last-error accessors wrapped**: `tst_get_last_error`
  (default `TstError::Internal as i32`) and `tst_get_last_error_str`
  (default: pointer to a static `b"\0"` byte slice). The static
  fallback on `_str` preserves the never-NULL contract documented
  in the rustdoc — a reentrant Drop double-borrow of the
  thread-local `RefCell` previously could panic out of `borrow()`.

- **6 inline unit tests** in `panic::tests` pin the contract:
  closure value passes through on success; panic returns the
  default and records `PanicCaught`; payload detail captured for
  both `&'static str` and formatted `String` payloads; null-ptr
  default works; void default works; open-path simulated panic
  (regression test for the architectural property).

No API change; no symbol changes. The `cargo public-api` baseline
stays byte-identical (the new `panic` module is private and the
`ffi_catch` helper is `pub(crate)`). The fix is binary-compatible
with existing linked C consumers.

---

### SRT RejectReason mapping fix (2026-05-11) — plan #49

SRT-01 from the 2026-05-10 audit
(`docs/analysis/2026-05-10-audit-slices/14-srt-bindings.md`).
`tst_srt::error::RejectReason::from_raw` previously mapped raw codes
`1001..=1014` as if they were the internal `SRT_REJECT_REASON` enum
offset by 1000 — they're actually the `SRT_REJX_*` HTTP-style
extension codes from `access_control.h`, which are set by remote
services via `srt_setrejectreason` and live in a different code
category entirely. Effects:

- Every libsrt-emitted handshake reject (bad passphrase, version
  mismatch, backlog, timeout, …) was reported as `Other(raw)`
  instead of its typed variant. The `tests/handshake.rs` integration
  test for passphrase mismatch was hitting its `eprintln!` "log it
  but don't fail" fallback because raw 10 (`SRT_REJ_BADSECRET`) was
  never matching the `BadSecret` arm.
- Conversely, an extension code 1001 (`SRT_REJX_KEY_NOTSUP` —
  StreamID key not supported) was being reported as `BadSecret`.

#### Fixed (breaking — `tst-srt`)

- **`tst_srt::error::RejectReason`** rewritten per `srt.h:535-558`
  (internal `SRT_REJC_INTERNAL`, ordinals 0..=17) and
  `access_control.h:21-71` (predefined `SRT_REJC_PREDEFINED`,
  1000..=1999):
  - **Removed** `ValueLearn` and `UnknownStreamId` — never existed
    in libsrt; mid-design guesses.
  - **Added** internal-category variants `Unknown` (0), `System` (1),
    `Peer` (2), `MessageApi` (12), `Congestion` (13), `Filter` (14),
    `Group` (15), `Timeout` (16), `Crypto` (17).
  - **Added** extension-category variants `Fallback` (1000),
    `KeyNotSupported` (1001), `Filepath` (1002), `HostNotFound`
    (1003), `Unauthorized` (1401), `Overload` (1402), `BadMode`
    (1405), `Unacceptable` (1406), `Conflict` (1409),
    `NotSupportedMedia` (1415), `Locked` (1423), `FailedDependency`
    (1424), `InternalServerError` (1500), `Unimplemented` (1501),
    `Gateway` (1502), `Down` (1503), `VersionUnsupported` (1505),
    `NoRoom` (1507).
  - **Behavioral rename** of existing variants: `BadSecret`,
    `Unsecure`, `Version`, `Resource`, `Rogue`, `Backlog`, `Ipe`,
    `Close`, `RdvCookie`, `BadRequest`, `Forbidden`, `NotFound`
    keep their identifiers but now map to the spec-correct raw
    codes (mostly small ordinals, not 1000+). Match-arms in
    downstream code still compile but the runtime category they
    catch shifts.
  - The enum remains `#[non_exhaustive]`; `Other(i32)` covers
    `SRT_REJC_USERDEFINED` (2000+) and unknown codes within either
    typed range.

- **`tst_srt::error::ConnectError::Rejected` sentinel check**
  shifted from `reason != RejectReason::Other(0)` to
  `reason != RejectReason::Unknown`. libsrt's `SRT_REJ_UNKNOWN`
  is raw 0, not 1000-and-something — the previous sentinel was
  consistent with the (broken) enum mapping and stops being
  meaningful after the fix.

- **`crates/srt-sys/wrapper.h`** now `#include`s
  `<srt/access_control.h>` so the 21 `SRT_REJX_*` constants are
  exposed as `pub const SRT_REJX_*: u32` in the generated
  bindings. `tst-srt` uses these to drift-detect upstream
  renumbering via the new
  `reject_reason_extension_named_constants` test.

---

### Video codec parser robustness fixes (2026-05-11) — plan #48

Three decoder-side robustness fixes from the 2026-05-10 audit
(`docs/analysis/2026-05-10-audit-slices/07-codec-h264.md` H264-01,
`.../16-codec-h266-vui-h274.md` H274-01 + H274-02). Library does not
encode H.264 or H.266, so the only behavior change is decoder-side:
malformed input that previously surfaced as `Ok(garbage)` now surfaces
as a typed `CodecParseError`.

#### Fixed (decoder behavior on malformed input)

- **`codec::h264::parse_sps`** — When the underlying `h264-reader`
  decoder surfaces `chroma_format_idc` outside the spec range (H.264
  V15 §7.4.2.1.1: shall be in 0..=3), `parse_sps` now returns
  `CodecParseError::ReservedValue { field: "chroma_format_idc", value }`
  instead of silently coercing the value to `Yuv420` and continuing.
  The previous behavior produced a `H264Sps` with crop offsets computed
  against the (different) original `chroma_format` and a fabricated
  chroma bit-depth.

- **`codec::h266::parse_sps`** — Now correctly consumes the optional
  `vui_payload(payloadSize)` tail per H.266 V4 §7.3.2.21 —
  `vui_parameters()` (H.274 §7.2) may not consume all `8 * payloadSize`
  bits, and the SPS caller must advance the cursor to the declared
  payload end before reading `sps_extension_flag`. Previously the
  parser mis-framed `sps_extension_flag` for any encoder that emitted
  the optional `vui_reserved_payload_extension_data` + marker +
  zero-pad tail. The `parse_h266_vui` `pub(super)` function signature
  also drops the unused `_payload_size_bytes` argument —
  tail-consumption is now correctly placed in the SPS caller.

- **`codec::h266` VUI parser** — `vui_chroma_sample_loc_type_frame`,
  `vui_chroma_sample_loc_type_top_field`, and
  `vui_chroma_sample_loc_type_bottom_field` are now validated against
  the H.274 V4 §7.3 (p. 20) range 0..=6 inclusive. Previously the
  parser used `read_ue()? as u8`, which silently accepted out-of-range
  values up to 255 and silently truncated values ≥ 256 to a valid
  in-range value (e.g. 256 → 0). All three sites now return
  `CodecParseError::ReservedValue` with the original `u32` value
  preserved.

#### Docs

- **`ColorInfo::chroma_loc`** rustdoc — H.274 V4 §7.3 (p. 20)
  inference rule documented: when `vui_chroma_loc_info_present_flag
  = 0` AND `ChromaFormatIdc == 1`, the spec infers
  `vui_chroma_sample_loc_type_frame = 6`. The parser leaves
  `chroma_loc = None` to preserve the "absent" vs "absent and inferred"
  distinction; callers needing the inferred value substitute 6
  themselves.

---

### MPEG-TS PSI multi-section reject + AV1 binding docs (2026-05-11) — plan #47

Two audit-driven fixes at the MPEG-TS layer from
`docs/analysis/2026-05-10-audit-slices/05-mpegts-demux.md` (DEMUX-01)
and `docs/analysis/2026-05-10-audit-slices/10-codec-av1.md` (AV1-05).

#### Public API

- **`klv::st0903`** unchanged. New variants additive on
  `#[non_exhaustive]` enums:
  - `mpegts::demux::event::NonConformantIssue::PsiMultiSectionUnsupported { pid, table_id, last_section_number }`
  - `mpegts::demux::psi::PsiParseError::MultiSectionUnsupported { table_id, last_section_number }`

#### Fixed

- **`mpegts::demux::psi`** — multi-section PSI tables
  (`last_section_number > 0` per H.222.0 §2.4.4.5) are now rejected
  with a new `PsiParseError::MultiSectionUnsupported` and surfaced as
  a typed `NonConformantIssue::PsiMultiSectionUnsupported` event.
  Prior behavior silently overwrote sibling sections via the
  version-dedup path, so a multi-section PMT delivered streams from
  only the last-arriving section. Full §2.4.4.5 reassembly is
  deferred until a consumer needs it (the corpus has zero
  multi-section captures — MISB-shaped ISR streams pack everything
  into a single section well under the 1021-byte short-form cap).
  Audit slice 05 finding DEMUX-01.

#### Docs

- **AV1 binding deviations** — `docs/deferred-features.md` AV1
  binding-§3.2/§3.4 carriage entry corrected: prior entry claimed
  `data_alignment_indicator=1` was not set; the muxer in fact does
  set it correctly for AV1 video PES. Entry reduced from three
  deviations to two (§3.2 framing + §3.4 stream_id). Inline rustdoc
  added at the AV1 PES writer site
  (`crates/tst-core/src/mpegts/mux/mod.rs`) pointing back to the
  deferred-features entry for the binding-deviation rationale.
  Audit slice 10 finding AV1-05.

---

### KLV follow-up — VMTI checksum + Security LS UL (2026-05-10) — plan #46

Two High-severity audit findings from `docs/analysis/2026-05-10-audit-slices/03-klv-other-sets.md` closed in this slice.

#### Public API

- **`klv::universal_label::UniversalLabel::SECURITY_LS_UL`** — new
  16-byte UL constant per MISB ST 0102.12 §6.7
  (`06.0E.2B.34.02.03.01.01.0E.01.03.03.02.00.00.00`, CRC 40980).
- **`klv::st0102::SECURITY_LS_UL`** — raw `[u8; 16]` re-export mirroring
  the `klv::st0903::VMTI_LS_UL` precedent. Used by consumers detecting
  the standalone (non-Tag-48-nested) Security LS carriage path.
- **`klv::st0903::encode_standalone(&VmtiLs, &mut [u8]) -> Result<usize, _>`**
  — new self-checksumming entry for standalone-VMTI carriage. Emits
  `[VMTI_LS_UL:16][outer BER length][body][Tag 1 checkSum TLV]` per
  ST 0903.4-17 / ST 0903.6-119. The Tag 1 value is the running 16-bit
  unsigned summation per §10.1.1.
- **`klv::st0903::encode_to_vec_standalone(&VmtiLs) -> Result<Vec<u8>, _>`**
  — convenience over `encode_standalone` allocating a fresh buffer.
- **`klv::st0903::encoded_len_standalone(&VmtiLs) -> usize`** — sizing
  helper for the standalone path.

#### Changed (wire-format)

- **`klv::st0903::encode` / `encode_to_vec`** is now exclusively the
  **embedded-VMTI body** entry — Tag 1 (checkSum) is silently dropped
  per ST 0903.6-120 ("where the VMTI LS is embedded-VMTI, the VMTI LS
  checkSum (Item 1) shall be omitted"). Any value the caller stored in
  `VmtiLs::checksum` is ignored. Callers who want a self-checksummed
  standalone-VMTI wire record use `encode_to_vec_standalone`. Decode is
  unchanged: `VmtiLs::checksum` still captures the Tag 1 value when
  present on the wire.

#### Tests

- Eight new regression tests pin the new contracts: `encode_omits_tag1_checksum_per_st0903_6_120`, `encode_drops_caller_supplied_checksum`, `encode_standalone_emits_tag1_last_per_st0903_4_17`, `encode_standalone_checksum_matches_running_sum_16`, `encode_standalone_round_trips_via_decode`, `encoded_len_standalone_matches_encode_standalone`, `security_ls_ul_canonical_bytes`, `security_ls_ul_reexport_matches_universal_label`.

---

### Pipeline close-flush and pairer PTS saturation fixes (2026-05-10) — plan #45

Three High-severity correctness fixes in `tst-pipeline` from the 2026-05-10
spec-validation audit (slice 13). No wire-format change — behavioral +
arithmetic fixes only.

#### Fixed (lifecycle / arithmetic)

- **`tst_pipeline::Sender::close`** — now best-effort flushes the
  buffered partial bundle before marking closed, matching `Drop`
  semantics (PIPE-01). Pre-fix, callers using the AutoCloseable /
  `__exit__` / `.use { }` / `tst_sender_close(...)` idioms could
  silently drop 1–6 partial TS packets sitting in `TsFraming::buffer`.

- **`tst_pipeline::MuxSender::close`** — now best-effort drains
  `pending_bytes` before marking closed, matching `Drop` semantics
  (PIPE-02). Pre-fix, queued back-pressure-buffered chunks were
  silently abandoned on explicit close. Cancel-first ordering is
  preserved (the `close_unblocks_parked_sender_thread` test continues
  to pass). Also: `close()` now gracefully handles a poisoned inner
  mutex via `if let Ok` instead of `.unwrap()`, parity with Drop.

- **`tst_pipeline::pairing::Pairer`** — nearest-mode arithmetic now
  uses `saturating_add` / `saturating_sub` at the three flagged sites
  in `nearest.rs` (PIPE-03 item 1). Pre-fix, PTS values approaching
  `i64::MAX` (theoretical, or from misconfigured sources) would
  overflow — panic in debug, silent wrap in release. The
  `pairing/mod.rs` module-doc has been rewritten (PIPE-03 item 2) to
  accurately describe the demuxer's PTS shape: per-event values
  bounded `0..(2^33 − 1)` per H.222.0 §2.4.3.7, with explicit
  semantics across the 33-bit rollover boundary.

#### Tests

- `close_flushes_buffered_partial_packets` pins the PIPE-01 contract
  via a Recorder transport.
- `close_drains_pending_bytes` pins the PIPE-02 contract via a
  `BackpressureOnce` transport with an external `Arc<Mutex<Vec<u8>>>`
  snoop slot.
- `close_does_not_panic_on_poisoned_lock` pins the poisoned-lock parity
  ride-along via a `PanicOnSend` transport.
- `near_i64_max_pts_does_not_overflow_buffered_drain` and
  `near_i64_max_pts_does_not_overflow_realtime_match` pin the PIPE-03
  arithmetic contract; both panic pre-fix in debug, both pass post-fix.

---

### KLV wire-format critical fixes (2026-05-10) — plan #44

Two wire-format-incompatible KLV defects from the 2026-05-10 spec-validation
audit. Both defects predate any external consumer; pre-1.0 break per the
break-freely policy.

#### Fixed (wire-format breaking)

- **`klv::imapb`** — encoder now writes unsigned big-endian per ST 1201.5
  §7.2.3 Table 1; previously emitted signed two's-complement, MSB-flipping
  every value. Also: truncate (not round) per §7.2.1 step 4a; proper
  `Zoffset = sF·a − floor(sF·a)` per §7.1.2 step 6 when the range straddles
  zero. Length cap widened from `1..=7` to `1..=8`. Affects every ST 0903
  VMTI emit and the VTargetPack IMAPB-encoded tags 10-16. Internal
  round-trips were previously consistent (encode + decode agreed on the
  wrong algorithm), masking the wire-format break. Supersedes the
  `length: 1..=7` claim from the Phase 6 entry below — Phase 6 introduced
  the typed error variants; plan #44 widens the cap to spec.

- **`klv::st0601::UasDatalinkLs`** — Tag 50 is now correctly typed as
  Platform Angle of Attack (int16 mapped to ±20°, sentinel `0x8000`) per
  ST 0601.19 §8.50; the previous "Platform Call Sign" typing was a
  misidentification. Platform Call Sign moves to Tag 59 (utf8 ≤ 127 B)
  per §8.59. New struct field: `platform_angle_of_attack_deg: Option<f64>`.
  Existing `platform_call_sign: Option<String>` field is preserved by
  name but now serializes to Tag 59. The `KlvEncodeError::StringTooLong`
  emitted for an over-length call sign now reports `tag: 59`.

#### Tests

- New ST 1201.5 spec-vector tests in `klv::imapb`: Appendix A Tests 2 + 3,
  ST 0903.6 §10.1.11 worked example (FOV 12.5° / 10.0° / 90.0°), and an
  L=8 round-trip.
- New ST 0601 wire-pin tests: `tag_50_is_platform_angle_of_attack_int16_per_spec`
  and `tag_59_is_platform_call_sign_utf8_per_spec`.
- Synthetic fixture `synthetic_full.klv` regenerated to exercise both
  Tag 50 and Tag 59 in the integration-test fixture-decode path.

#### Substrate cleanup

- `klv::imapb::ImapbParams` lost private `scale()` + `signed_offset()`
  methods; gained `sf()` + `z_offset()` per ST 1201.5 §8.9 Summary.
- Dead `write_signed_be` + `read_signed_be` helpers removed (audit
  finding KLV-SUB-09 — `1u64 << 64` UB risk at n=8 obviated by deletion).

---

### Phase 6 — Test infrastructure (2026-05-10)

Test-infrastructure improvements and two latent-bug fixes surfaced
by the new property tests. 12 commits `c94881d..016c3e4`.

#### Public API

Two latent substrate issues surfaced by Phase 6's new property tests
fixed at the type level (pre-1.0 break per the break-freely policy):

- **KLV ST 0102 / ST 0903 PartialEq:** `SecurityLs::eq`, `VmtiLs::eq`,
  and `VTargetPack::eq` no longer compare `field_errors`. Two LSes
  that produced identical field values are now semantically equal
  regardless of which fields failed strict decode. `field_errors`
  is a decoder-side diagnostic, not part of the LS value.
  `PartialEq` trait surface unchanged; `StructuralPartialEq` impls
  removed (auto-generated by derive but not by manual impl).
- **IMAPB length cap:** `encode_imapb` and `decode_imapb` now reject
  `length` not in `1..=7` with new typed variants
  `KlvEncodeError::UnsupportedImapbLength` and
  `KlvFieldError::UnsupportedImapbLength`. Previously, `length >= 8`
  caused `i64` overflow in `ImapbParams::signed_offset` — panic in
  debug, silent wrap in release. ST 1201.5 defines IMAPB for any
  L-byte width; this is an internal-arithmetic limitation. Both
  error enums are `#[non_exhaustive]`; the addition is
  forwards-compatible.

#### Test infrastructure

- **Loopback probe + atomic-signal helpers** in `crates/tst-srt/tests/common/mod.rs`:
  `loopback_probe()`, `require_loopback!()` macro, `wait_for_ready(&AtomicBool)`.
- **28 default-running loopback tests probe-gated** via `require_loopback!()`
  across 19 files. Sandbox/restricted CI environments now emit
  `SKIP: loopback unavailable` instead of failing dozens of tests.
- **7 listener-settle sites migrated** from `thread::sleep(50ms)`
  to `wait_for_ready(&AtomicBool)` atomic-signal poll. Matches the
  `accept_done` precedent from `cancellation_loopback.rs`. Bonus:
  fixes a previously-flaky `accept_timeout_succeeds_when_peer_connects`
  test by eliminating the race window.
- **H.266 parameter-set parsers added** to the existing
  `parse_parameter_sets` fuzz target (target count stays at 16).
- **KLV ST 0102 + ST 0903 fuzz targets** upgraded from panic-only
  to decode→encode→decode round-trip identity. ST 0102 filters
  inputs containing multi-byte BER-OID unknown tags (documented
  encoder limitation). Codec parameter sets (H.264/H.265/H.266) and
  AV1 sequence header stay panic-only — no encoder counterpart.
- **6 new property tests:**
  - `tst-core/tests/klv_proptest.rs`: BER round-trip, BER-OID round-trip,
    IMAPB round-trip (lerp value generation, scale-factor tolerance
    with f64 ULP floor).
  - `tst-core/tests/mpegts_psi_proptest.rs`: PSI mux→demux round-trip,
    descriptor build/parse round-trip, `Demuxer::feed` chunking
    invariance.
- **3 new CI rails:**
  - `cargo test --workspace --all-features` (closes feature-matrix gap).
  - `linux-musl-x86_64` (tst-core + tst-pipeline scope; libsrt-bound
    crates need a deeper rework for musl-native libsrt).
  - Nightly fuzz compile smoke (`cargo +nightly fuzz check` for both
    fuzz crates).

Test count: 1320 → 1326 default-features (+6: 3 KLV proptests + 3 PSI
proptests).

---

### Phase 5 — Internal hygiene (2026-05-10)

God-module splits, test-helper de-duplication, fuzz-target relocation,
focused dead-code sweep. 15 commits `7ab2ffb..2709572`.

#### Public API

The four moved-types' canonical paths shifted (user-facing re-exports
preserved):

- `tst_core::mpegts::mux::*` types: `VideoCodec`, `KlvStreamType`,
  `AudioCodec`, `SubtitleCodec`, `StreamKind`, `TeletextField`,
  `StreamSpec`, `VideoStreamHandle`, `KlvStreamHandle`,
  `AudioStreamHandle`, `SubtitleStreamHandle` now resolve via
  `mpegts::mux::types::*` (cargo-public-api visible canonical path).
  User-facing `tst_core::mpegts::mux::*` re-exports unchanged.
- `tst_core::mpegts::mux::*` configuration types: `MuxerConfig`,
  `MuxerConfigBuilder`, `MuxerProgramConfig`, `MuxerProgramConfigBuilder`
  now resolve via `mpegts::mux::config::*`. User-facing paths unchanged.
- `tst_core::mpegts::demux::*`: `DemuxerStats`, `DemuxerOptions`,
  `DemuxerBuilder`, `ProgramTracker` now resolve via
  `mpegts::demux::types::*`. User-facing paths unchanged.
- `tst_core::codec::h265::bitreader` → `tst_core::codec::bitreader`
  (codec-agnostic; Annex-B reader is consumed by both H.265 and H.266
  parsers). `BitReader` is `#[doc(hidden)]` since Phase 3.6.1; not
  user-facing.

`klv::pack::Iter` retained as `#[doc(hidden)] pub` (the audit's claim
that fuzz-target relocation enables a `pub → pub(crate)` tightening was
structurally incorrect — `cargo-fuzz` creates a separate crate; tightening
remains gated on either a `#[cfg(fuzzing)] iter_for_fuzz` entry point or
deletion of the `klv_iter` fuzz target).

#### Internal restructure (Phase 5)

- **`mpegts::mux::types`** (NEW, ~485 LoC): codec/stream-class enums,
  `StreamSpec`, four opaque stream-handle types extracted from
  `mpegts/mux/mod.rs`. Re-exported via `pub use types::*;`.
- **`mpegts::mux::config`** (NEW, ~873 LoC): `MuxerProgramConfig`,
  `MuxerConfig`, `MuxerConfigBuilder`, `MuxerProgramConfigBuilder`
  extracted; private `validate_language_code` helper migrated alongside.
- **`mpegts::common::handle_pack`** (NEW): four byte-near-identical
  `pack` / `unpack` impls on `Video`/`Klv`/`Audio`/`SubtitleStreamHandle`
  collapse to one shared substrate. Defensive `& WITHIN_MASK` form
  applied uniformly (was already present in Audio/Subtitle; behavior
  identical on valid inputs, slightly safer in release on out-of-range).
- **`mpegts::demux::types`** (NEW, ~152 LoC): `DemuxerStats`,
  `DemuxerOptions`, `ProgramTracker`, `DemuxerBuilder` extracted from
  `mpegts/demux/demuxer.rs`. Private `DEFAULT_PES_CAP_*` constants
  consolidated into the existing `pub(crate) const fn` accessors.
- **`codec::bitreader`** (PROMOTED from `codec::h265::bitreader`):
  Annex-B Exp-Golomb reader is consumed by both `codec::h265::*` and
  `codec::h266::*`. File-level `#[allow(dead_code)]` removed.
  `BitReader::bit_cap` field and `BitReader::at_end()` method deleted
  (no consumers).
- **`tst-test-helpers`** (NEW workspace member, `publish = false`):
  consolidates `synthetic_nal` (47 LoC) + `ts_parser` (218 LoC) +
  `mock_transport` (82 LoC) — three modules previously byte-identical
  across `tst-core/tests/common/`, `tst-pipeline/tests/common/`, and
  `tst-srt/tests/common/`. ~10 consumer test files swap their import
  paths to `tst_test_helpers::*`. `tst-core` and `tst-pipeline` gain
  the dev-dep; `tst-srt` already had it from Task 8.
- **`crates/tst-core/fuzz/`** (NEW cargo-fuzz crate): hosts 15 of 16
  fuzz targets. `tst-srt/fuzz/` retains only `url_parse` (URL parsing
  lives in `tst-srt`); the `tst-core` dep dropped from
  `tst-srt/fuzz/Cargo.toml`. Six corpus subdirectories moved alongside.
  `mux_push_klv` arity fix landed during the relocation
  (`push_klv(data, 0)` → `push_klv(data, 0, 0)` — pre-existing
  breakage from plan #30's `metadata_service_id` addition).
- **Dead-code annotations**: 8 module-level `#![allow(dead_code)]`
  before; 4 after. 3 file-level annotations removed; 1 confirmed-dead
  item deleted (`Av1BitReader::buf_len_bits`); 2 narrow per-item
  allows added (`Av1BitReader::byte_align`, `bit_pos` — used only by
  inline `#[cfg(test)]` blocks; clippy can't see test consumers). The
  `tst-srt/tests/common/mod.rs` and bindgen-generated `srt-sys/src/lib.rs`
  annotations left in place. (Audit estimated 43 annotations; reality
  was 8 — most already swept by prior phases incidentally.)
- **Orphan dirs deleted**: `crates/tst-core/tests/fixtures/{h266,av1}/`
  (each contained only a stale `regen.sh` producing files no test
  reads; canonical fixtures at `tests/fixtures/codec/{h266,av1}/`),
  and `crates/tst-srt/fuzz/corpus/klv_st1910_unwrap/` (target deleted
  in plan #25's AU cell rework; corpus subdir was missed).

#### File size deltas

- `mpegts/mux/mod.rs`: 5489 → 4151 LoC (-1338, -24%).
- `mpegts/demux/demuxer.rs`: 3290 → 3158 LoC (-132).
- `codec/h265/bitreader.rs` (219 LoC) → `codec/bitreader.rs` (211 LoC,
  -8 from dead-item deletions).

#### Tests

- 1320 passing on default features (matches pre-Phase-5 baseline).
- 1319 passing on `--no-default-features` (matches prior Phase 3-Phase 4
  numbers).
- All 3 Phase 3 CI ratchets clean
  (`check-c-abi-rustdoc-coverage`, `check-close-contract-presence`,
  `check-no-public-usize`).
- Public-API baselines refreshed for `tst-core` and `tst-pipeline` to
  reflect the moved-types canonical-path renames; `tst-srt` baseline
  unchanged.
- `cargo public-api` surface is unchanged at user-facing-paths level
  (re-exports via `mpegts::mux::*` / `mpegts::demux::*` preserve every
  caller-visible import).

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
  `ManagedRecvTransport` (documented at the struct level, 3 sites);
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
