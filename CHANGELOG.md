# Changelog

All notable changes to this project are documented in this file.

Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) style.
Versioning: [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
and plan #48 (video codec parser robustness fixes) also ride this
release.

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
