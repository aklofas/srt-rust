# Codex Waves 1-6 Comprehensive Rereview Report

Date: 2026-05-19
Reviewer: Codex, static audit only
Review request: `docs/refactor-1/_claude-prompt-to-codex-waves-1-6-comprehensive-rereview.md`
Reviewed repository root: `/home/aklofas/Projects/ts-transformer/ts-transformer`
Reviewed HEAD: `81aa589 docs: CHANGELOG entry for Codex Wave 6 validation fixes (plan #92)`

## Scope And Constraints

This review was performed as a static audit. I did not run tests, builds, generators, formatters, fuzzers, or examples. I did not modify source code.

The review focused on the prompt's requested Waves 1-6 hotspots, especially:

- C ABI error projection and last-error invariants.
- C header sectioning and cbindgen drift ratchets.
- KLV synchronous metadata AU-cell behavior and documentation.
- PTS and `StreamTypeCode` public boundary changes.
- Managed transport close/cancel/reconnect semantics.
- `MuxError` wildcard and public projection behavior.
- Public API baselines, CI ratchets, and stale docs/examples.

## Executive Summary

Plan #92's targeted fixes are mostly present by static inspection:

- `tstrans.h` now has 9 unique generated section dividers.
- `scripts/check-c-header-section-uniqueness.sh` is wired as a CI ratchet.
- `MuxError::kind()` now has an internal wildcard fallback.
- The mux constructor extraction into `state.rs` appears to preserve descriptor-cache and stats initialization behavior.

However, I found one high-confidence behavioral C ABI issue and two documentation/API contract issues that should be fixed before treating the Waves 1-6 rereview as fully clean.

## Required Finding 1: C ABI `TST_E_NOT_AVAILABLE` Paths Leave Stale Last-Error State

Severity: High
Area: C ABI error projection
Prompt hotspot: G, C ABI error mappers and output initialization

### Problem

Several `#[no_mangle]` C ABI functions return `TstError::NotAvailable as i32` directly without recording a fresh last-error value. That means callers can receive `TST_E_NOT_AVAILABLE` while `tst_get_last_error()` still describes an older, unrelated failure.

This violates the expected C ABI invariant that negative return paths update last-error state through `set_last_error`, `record_*`, or an equivalent helper.

### High-Confidence Examples

Plain mux sender socket stats:

- File: `crates/tst-c/src/sender/mux_sender.rs`
- Line: 474
- Function: `tst_mux_sender_get_socket_stats`
- Observed behavior: `None => TstError::NotAvailable as i32`
- Issue: returns a negative code without setting last-error.

Managed mux sender socket stats:

- File: `crates/tst-c/src/sender/mux_sender.rs`
- Line: 1081
- Function: `tst_managed_mux_sender_get_socket_stats`
- Observed behavior: `None => TstError::NotAvailable as i32`
- Issue: returns a negative code without setting last-error.

Plain TS receiver socket stats:

- File: `crates/tst-c/src/receiver/ts_receiver.rs`
- Line: 290
- Function: `tst_receiver_get_socket_stats`
- Observed behavior: `None => TstError::NotAvailable as i32`
- Issue: returns a negative code without setting last-error.

Managed TS receiver socket stats:

- File: `crates/tst-c/src/receiver/ts_receiver.rs`
- Line: 577
- Function: `tst_managed_receiver_get_socket_stats`
- Observed behavior: `None => TstError::NotAvailable as i32`
- Issue: returns a negative code without setting last-error.

### Additional Static Search Hits To Audit

Static search for direct `TstError::NotAvailable as i32` returns found these files and lines:

- `crates/tst-c/src/receiver/ts_receiver.rs:290`
- `crates/tst-c/src/receiver/ts_receiver.rs:577`
- `crates/tst-c/src/sender/ts_sender.rs:180`
- `crates/tst-c/src/sender/ts_sender.rs:387`
- `crates/tst-c/src/receiver/demux_receiver/managed.rs:387`
- `crates/tst-c/src/receiver/demux_receiver/stats.rs:67`
- `crates/tst-c/src/receiver/raw_receiver.rs:228`
- `crates/tst-c/src/receiver/raw_receiver.rs:617`
- `crates/tst-c/src/sender/raw_sender.rs:321`
- `crates/tst-c/src/sender/raw_sender.rs:396`
- `crates/tst-c/src/sender/mux_sender.rs:474`
- `crates/tst-c/src/sender/mux_sender.rs:1081`

The fixing agent should inspect every hit and either:

- add a fresh last-error recording call before returning `TST_E_NOT_AVAILABLE`, or
- document why the function is not an error-returning ABI surface, if any such exception exists.

### Recommended Fix

Add a shared helper such as:

```rust
fn record_not_available(message: impl Into<String>) -> i32 {
    set_last_error(TstError::NotAvailable, message);
    TstError::NotAvailable as i32
}
```

Use the existing local error helper style rather than introducing a parallel convention if the module already has one.

Then replace direct `TstError::NotAvailable as i32` returns in C ABI functions with the helper or direct `set_last_error(...)` calls.

### Recommended Validation

Add targeted C ABI tests proving that:

- A previous unrelated last-error is overwritten.
- The function returns `TST_E_NOT_AVAILABLE`.
- `tst_get_last_error()` or the equivalent last-error accessor reports a NotAvailable error with a relevant message.

At minimum, cover one plain sender/receiver socket stats accessor and one managed accessor. Preferably add a ratchet that flags direct `TstError::NotAvailable as i32` returns in `crates/tst-c/src`.

## Required Finding 2: Single-Stream KLV C ABI Docs Omit Raw Payload / No Pre-Wrapped AU Cell Contract

Severity: Medium
Area: C ABI documentation and binding safety
Prompt hotspot: E, KLV semantics and docs

### Problem

The multi-stream `_to` C ABI KLV functions document synchronous metadata behavior, but the single-stream KLV functions either have only a short generic comment or no generated header comment explaining the payload contract.

For synchronous metadata, callers must pass raw MISB Local Set bytes. They must not pre-wrap the payload in a Metadata Access Unit Cell. The muxer performs AU-cell wrapping internally.

This is an important binding-facing contract because pre-wrapped input can produce invalid or double-wrapped metadata.

### Affected Surfaces

Muxer single-stream KLV push:

- Source: `crates/tst-c/src/sender/muxer.rs:85`
- Function: `tst_muxer_push_klv`
- Current source doc: "Push one pre-built KLV blob."
- Generated header: `crates/tst-c/include/tstrans.h:1954`
- Issue: does not clearly state raw LS bytes/no pre-wrapped AU cell behavior.

Plain mux sender single-stream KLV send:

- Source: `crates/tst-c/src/sender/mux_sender.rs:130`
- Function: `tst_mux_sender_send_klv`
- Generated header: `crates/tst-c/include/tstrans.h:1237`
- Issue: no equivalent single-stream KLV contract in the generated header.

Managed mux sender single-stream KLV send:

- Source: `crates/tst-c/src/sender/mux_sender.rs:772`
- Function: `tst_managed_mux_sender_send_klv`
- Issue: no equivalent single-stream KLV contract in the generated header.

### Recommended Fix

Add rustdoc to all single-stream KLV C ABI entry points mirroring the `_to` function contract:

- Payload must be raw KLV Local Set bytes.
- For synchronous KLV streams, the muxer wraps payloads into Metadata AU Cells internally.
- Callers must not pre-wrap Metadata AU Cells.
- The timestamp is the presentation timestamp in the expected 90 kHz domain.
- `metadata_service_id` behavior should be stated wherever the function accepts or derives it.

Regenerate `tstrans.h` after source doc updates and confirm the generated comments appear in the relevant header sections.

### Recommended Validation

Static validation is enough for this specific doc issue:

- Confirm the three source functions above have explicit rustdoc.
- Confirm generated `tstrans.h` carries the contract for each function.
- Confirm docs/examples do not show pre-wrapped AU-cell payloads being passed into these APIs.

## Required Finding 3: User-Facing Docs Still Contain Stale API Names And Signatures

Severity: Medium
Area: documentation accuracy after Waves 1-6 public API changes
Prompt hotspot: I, docs/examples stale after breaking changes

### Problem

Several user-facing docs still describe pre-refactor API shapes. These are not implementation bugs, but they will mislead users and future agents validating the Waves 1-6 API state.

### Specific Stale References

`docs/guide-mpegts-mux.md`

- Line 302 still describes mux APIs using raw `i64` PTS parameters:
  - `push_video(nal: &[u8], pts_90khz: i64, key_frame: bool)`
  - `push_klv(... pts_90khz: i64 ...)`
- These should reflect typed `Pts90khz` usage.

`docs/guide-mpegts-mux.md`

- Line 371 shows:
  - `mux.push_klv(&klv, Pts90khz::new(0))?;`
- Current API requires the metadata service id argument as well.

`docs/guide-klv.md`

- Line 201 references `EncodeOptions`.
- Current code uses `EncodeConfig` in `crates/tst-core/src/klv/st0601/model.rs`.

Pairing docs still refer to old `pipeline::pairing` naming:

- `docs/architecture.md:92`
- `docs/guide-pipeline.md:645`
- `docs/guide-mpegts-demux.md:409`
- `docs/guide-mpegts-demux.md:666`
- `docs/deferred-features.md:543`
- `docs/deferred-features.md:551`

The shipped path appears to be `tst_pipeline::ext::pairing::Pairer`, and the docs should consistently say that.

### Recommended Fix

Update docs to match current public APIs:

- Replace raw public PTS signatures with `Pts90khz`.
- Add missing `metadata_service_id` arguments to KLV mux examples.
- Replace `EncodeOptions` with `EncodeConfig` where the docs are describing current API usage.
- Replace stale `pipeline::pairing` references with `tst_pipeline::ext::pairing`, except where a historical note explicitly needs to mention the old name.

### Recommended Validation

After doc edits, statically search for old API names and signatures:

- `EncodeOptions`
- `pts_90khz: i64`
- `push_klv(&klv, Pts90khz::new(0))`
- `pipeline::pairing`

Some references in `docs/conventions.md` are intentionally historical "bad naming" examples and should not be changed blindly.

## Residual Risk: C ABI Layout Assert Policy Is Selective

Severity: Low / policy clarification
Area: C ABI layout drift
Prompt hotspot: G

The cbindgen trailer currently asserts selected ABI-sensitive struct sizes, including:

- `tst_nal_t`
- `tst_obu_t`
- `tst_descriptor_t`
- `tst_stream_info_t`
- `tst_klv_link_t`
- `tst_demux_receiver_stats_t`
- `tst_event_t`
- `tst_stream_codec_stats_t`
- `tst_socket_stats_t`

There are additional C-visible `#[repr(C)] pub struct` types in `crates/tst-c/src/event.rs` and `crates/tst-c/src/stats.rs` that do not have corresponding `_TST_ABI_ASSERT` entries.

I am not treating this as a blocker because Plan #92 specifically targeted adding the missing `tst_socket_stats_t` assertion to the existing pinned set. Still, the project should clarify whether every C-visible `tst_*` struct requires a static size assertion or whether only selected layout-sensitive structs are intentionally pinned.

Recommended follow-up:

- Document the policy in `cbindgen.toml` near the trailer.
- If the stricter policy is desired, add asserts for all C-visible `tst_*` repr(C) structs and a ratchet that checks coverage.

## Static Areas Reviewed With No High-Confidence Blocking Findings

### C Header Sectioning

Reviewed:

- `crates/tst-c/build.rs`
- `crates/tst-c/tests/header_drift.rs`
- `scripts/check-c-header-section-uniqueness.sh`
- `crates/tst-c/include/tstrans.h`

Static result:

- The generated header now has exactly 9 unique section dividers.
- The build script and test path appear to mirror the same sectioning algorithm.
- The CI ratchet for section uniqueness is wired.

No high-confidence issue found.

### Mux Constructor Extraction

Reviewed:

- `crates/tst-core/src/mpegts/mux/mod.rs`
- `crates/tst-core/src/mpegts/mux/state.rs`

Static result:

- `mod.rs` is now approximately 320 lines.
- Extracted helpers in `state.rs` preserve the observed descriptor-cache ordering:
  - KLVA
  - AV01
  - AC-3
  - ISO-639
  - subtitle descriptors
  - caller-provided descriptors
- Stats initialization appears preserved relative to the pre-extraction code.

No high-confidence issue found.

### `MuxError` Wildcard And Projection

Reviewed:

- `crates/tst-core/src/error.rs`
- `crates/tst-c/src/sender/mux_error.rs`
- `crates/tst-pipeline/src/shell_error.rs`
- `scripts/check-raw-c-mapper-coverage.sh`
- `scripts/check-mux-error-kind-coverage.sh`

Static result:

- `MuxError::kind()` has an explicit wildcard fallback to internal error semantics.
- The C ABI raw mapper ratchet exists.
- The outer `ShellErrorKind` projection remains intentionally coarser and maps many mux usage/config variants to `ConfigInvalid`, matching the documented decision that the pipeline shell tier collapses some distinctions.

No high-confidence issue found.

### Managed Transport Close / Cancel / Reconnect Semantics

Reviewed:

- `crates/tst-core/src/transport.rs`
- `crates/tst-srt/src/transport.rs`
- `crates/tst-pipeline/src/managed_receive.rs`
- `crates/tst-pipeline/src/reconnect/mod.rs`

Static result:

- `ManagedRecvTransport` tracks `closed`, `explicit_close`, and `cancelled`.
- Caller-driven close/cancel paths appear to preserve explicit-close semantics.
- `ManagedTransport::send_managed` does not appear to hold the transport lock across reconnect in a way that creates an obvious static deadlock.

No high-confidence issue found.

### PTS And `StreamTypeCode`

Reviewed:

- `crates/tst-core/src/mpegts/common/mod.rs`
- `crates/tst-core/src/mpegts/mux/pes.rs`
- `crates/tst-core/src/mpegts/demux/pes_emit.rs`
- C ABI sender/receiver event conversion paths by static search

Static result:

- Public PTS construction uses `Pts90khz`.
- MPEG-TS emission masks to 33 bits before serializing.
- `StreamTypeCode` preserves unknown stream-type bytes through `from_byte` / `as_byte`.

No high-confidence issue found.

### KLV Runtime AU-Cell Behavior

Reviewed:

- `crates/tst-core/src/mpegts/mux/push_klv.rs`

Static result:

- Synchronous metadata streams are wrapped into Metadata AU Cells.
- Private-data KLV streams are not wrapped.
- Payload length checks account for AU-cell overhead.

No high-confidence runtime issue found. The remaining KLV issue is documentation coverage for single-stream C ABI entry points, listed above.

### Public API Baselines And CI Wiring

Reviewed:

- `.github/workflows/ci.yml`
- public API baseline files by static search
- recent git history for public API baseline updates

Static result:

- Public API drift checks are wired in CI.
- Recent baseline refresh commits exist for the Wave 6 public API changes.
- CI also wires the raw C mapper, mux error kind, and C header section uniqueness ratchets.

No high-confidence issue found without running the checks.

## Suggested Prompt For The Fixing Agent

Use this prompt for the next implementation agent:

```text
You are fixing issues from docs/refactor-1/_codex-waves-1-6-comprehensive-rereview-report.md.

Constraints:
- You may edit source, tests, generated headers, and docs as needed.
- Preserve existing project style and helper conventions.
- Run the narrowest relevant tests/checks after edits if the environment allows.

Required fixes:
1. Audit every direct `TstError::NotAvailable as i32` return in `crates/tst-c/src`. For any C ABI function returning a negative code, ensure last-error is freshly set before returning `TST_E_NOT_AVAILABLE`. Prefer a shared helper if it matches local style. Add targeted tests proving stale last-error is overwritten.
2. Add explicit C ABI rustdoc for single-stream KLV entry points:
   - `tst_muxer_push_klv`
   - `tst_mux_sender_send_klv`
   - `tst_managed_mux_sender_send_klv`
   The docs must say synchronous KLV callers pass raw MISB Local Set bytes and must not pre-wrap Metadata AU Cells. Regenerate `tstrans.h`.
3. Update stale docs:
   - Replace current API docs that still show `pts_90khz: i64` with `Pts90khz`.
   - Fix `push_klv(&klv, Pts90khz::new(0))` examples to include `metadata_service_id`.
   - Replace current-use `EncodeOptions` references with `EncodeConfig`.
   - Replace stale `pipeline::pairing` current-use references with `tst_pipeline::ext::pairing`, while preserving explicitly historical examples if any.

Recommended validation:
- Search for remaining direct `TstError::NotAvailable as i32` returns and justify any that remain.
- Search generated `tstrans.h` for the new KLV raw/no-prewrap language on all three single-stream functions.
- Search docs for `EncodeOptions`, `pts_90khz: i64`, `push_klv(&klv, Pts90khz::new(0))`, and `pipeline::pairing`; verify remaining hits are intentionally historical.
- Run the relevant C ABI tests, header drift checks, and docs/static ratchets if available.
```

