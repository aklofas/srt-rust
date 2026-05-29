# tst-core integration-test movement map

The 61 former top-level `tests/*.rs` integration binaries were consolidated
into **6 domain harnesses** to cut per-file binary link/startup overhead. Each
domain is now a single binary `tests/<domain>.rs` that `#[path]`-includes its
member files from `tests/<domain>/`.

## What changed (and what did not)

- **Test bodies are unchanged.** This was a pure relocation: `git mv` plus a
  thin `mod` aggregator per domain. No assertions, fixtures, or `#[cfg]` gates
  were edited.
- **Fully-qualified test paths gained a `<domain>::<file>::` prefix** because
  each former file is now a module inside its domain binary (e.g.
  `h264_async_klv_roundtrip` → `mpegts::demux::h264_async_klv_roundtrip`).
  Filtering still works: `cargo test -p tst-core --test mpegts demux::`.
- **A few member files were renamed** for clarity: the `mpegts_` / `klv_`
  filename prefixes were dropped inside their domain dirs, and the five
  `wave_i*` files were given content-descriptive names (see table).
- A small number of files needed mechanical path fixes after moving one
  directory deeper: relative `include_bytes!("fixtures/…")` became
  `"../fixtures/…"`, and `mod common;` became
  `#[path = "../common/mod.rs"] mod common;`. (`env!("CARGO_MANIFEST_DIR")`-
  anchored and CWD-relative paths were unaffected.)
- Note: the `conformance` domain contains a member also named `conformance`,
  so `tests/conformance/conformance.rs` is the former `tests/conformance.rs`
  and `tests/conformance.rs` is now the aggregator.

## Equivalence check

Verified the consolidation added/dropped/renamed **no test**: the whole-crate
`cargo test -- --list` count is unchanged (1399), and the multiset of test
leaf-names (the segment after the last `::`, invariant under the module-prefix
change) is byte-identical before and after — for both the active list and the
`--ignored` list, in both feature modes.

## Movement table

### `mpegts/` — MPEG-TS container mux/demux behavior (PSI, PES, AU cells, descriptors, subtitle/audio carriage). Pure tst-core — no transport.

| old `tests/…` | new `tests/…` |
| --- | --- |
| `mpegts_au_cell_round_trip.rs` | `mpegts/au_cell_round_trip.rs` |
| `mpegts_au_cell_tolerance.rs` | `mpegts/au_cell_tolerance.rs` |
| `mpegts_au_reassembly.rs` | `mpegts/au_reassembly.rs` |
| `mpegts_audio_fixtures.rs` | `mpegts/audio_fixtures.rs` |
| `mpegts_audio_treat_as.rs` | `mpegts/audio_treat_as.rs` |
| `mpegts_demux.rs` | `mpegts/demux.rs` |
| `mpegts_demux_audio.rs` | `mpegts/demux_audio.rs` |
| `mpegts_demux_caps.rs` | `mpegts/demux_caps.rs` |
| `mpegts_demux_multi_program.rs` | `mpegts/demux_multi_program.rs` |
| `mpegts_demux_pes_validation.rs` | `mpegts/demux_pes_validation.rs` |
| `mpegts_demux_robustness.rs` | `mpegts/demux_robustness.rs` |
| `mpegts_demux_strict.rs` | `mpegts/demux_strict.rs` |
| `mpegts_demux_subtitle.rs` | `mpegts/demux_subtitle.rs` |
| `mpegts_mux.rs` | `mpegts/mux.rs` |
| `mpegts_mux_audio.rs` | `mpegts/mux_audio.rs` |
| `mpegts_mux_builder_errors.rs` | `mpegts/mux_builder_errors.rs` |
| `mpegts_mux_demux_audio_roundtrip.rs` | `mpegts/mux_demux_audio_roundtrip.rs` |
| `mpegts_mux_demux_subtitle_roundtrip.rs` | `mpegts/mux_demux_subtitle_roundtrip.rs` |
| `mpegts_mux_descriptor_invariant.rs` | `mpegts/mux_descriptor_invariant.rs` |
| `mpegts_mux_descriptors_roundtrip.rs` | `mpegts/mux_descriptors_roundtrip.rs` |
| `mpegts_mux_dvb_subtitle_pes.rs` | `mpegts/mux_dvb_subtitle_pes.rs` |
| `mpegts_mux_dvb_teletext_pes.rs` | `mpegts/mux_dvb_teletext_pes.rs` |
| `mux_error_kind_routing.rs` | `mpegts/mux_error_kind_routing.rs` |
| `mpegts_mux_klv_pes.rs` | `mpegts/mux_klv_pes.rs` |
| `mpegts_mux_multi_program.rs` | `mpegts/mux_multi_program.rs` |
| `mpegts_mux_multi_stream.rs` | `mpegts/mux_multi_stream.rs` |
| `mpegts_mux_proptest.rs` | `mpegts/mux_proptest.rs` |
| `mux_shorthand_multi_program.rs` | `mpegts/mux_shorthand_multi_program.rs` |
| `mpegts_mux_subtitle.rs` | `mpegts/mux_subtitle.rs` |
| `mpegts_psi_proptest.rs` | `mpegts/psi_proptest.rs` |
| `mpegts_subtitle_fixtures.rs` | `mpegts/subtitle_fixtures.rs` |
| `mpegts_subtitle_treat_as.rs` | `mpegts/subtitle_treat_as.rs` |

### `klv/` — KLV substrate (BER/BER-OID/IMAPB) and typed MISB sets (ST 0601/0102/0605/0903). Pure tst-core metadata.

| old `tests/…` | new `tests/…` |
| --- | --- |
| `klv_proptest.rs` | `klv/proptest.rs` |
| `klv_st0102_via_st0601.rs` | `klv/st0102_via_st0601.rs` |
| `klv_st0601.rs` | `klv/st0601.rs` |
| `klv_st0903_standalone_ul.rs` | `klv/st0903_standalone_ul.rs` |
| `klv_st0903_via_st0601.rs` | `klv/st0903_via_st0601.rs` |
| `klv_typed_set_proptest.rs` | `klv/typed_set_proptest.rs` |

### `codec/` — Elementary-stream codec parsing and carriage (H.264/H.265/H.266, AV1, MPEG/AAC audio). Pure tst-core.

| old `tests/…` | new `tests/…` |
| --- | --- |
| `audio_corpus_cross_check.rs` | `codec/audio_corpus_cross_check.rs` |
| `audio_frame_roundtrip.rs` | `codec/audio_frame_roundtrip.rs` |
| `av1_carriage_roundtrip.rs` | `codec/av1_carriage_roundtrip.rs` |
| `av1_codec_integration.rs` | `codec/av1_codec_integration.rs` |
| `av1_no_panic.rs` | `codec/av1_no_panic.rs` |
| `codec_parameter_sets.rs` | `codec/codec_parameter_sets.rs` |
| `codec_stats.rs` | `codec/codec_stats.rs` |
| `codec_stats_mux.rs` | `codec/codec_stats_mux.rs` |
| `h265_real_x265_round_trip.rs` | `codec/h265_real_x265_round_trip.rs` |
| `h266_carriage_roundtrip.rs` | `codec/h266_carriage_roundtrip.rs` |
| `h266_codec_integration.rs` | `codec/h266_codec_integration.rs` |
| `h266_real_encoder_round_trip.rs` | `codec/h266_real_encoder_round_trip.rs` |

### `conformance/` — Conformance-corpus parsing plus demux error/recovery discrimination. Pure tst-core.

| old `tests/…` | new `tests/…` |
| --- | --- |
| `conformance.rs` | `conformance/conformance.rs` |
| `demux_error_discrimination.rs` | `conformance/demux_error_discrimination.rs` |
| `demux_malformed_pes_recovery.rs` | `conformance/demux_malformed_pes_recovery.rs` |

### `regression/` — Cross-implementation interop regressions (output shape vs external decoders/tools). Exercise tst-core mux/demux/KLV output only — no transport.

| old `tests/…` | new `tests/…` |
| --- | --- |
| `wave_i2_av1_external_decoder.rs` | `regression/av1_external_decoder.rs` |
| `wave_i3_ber_oid_symmetry.rs` | `regression/ber_oid_symmetry.rs` |
| `wave_i3_imapb_spec_vectors.rs` | `regression/imapb_spec_vectors.rs` |
| `wave_i3_st0903_incremental.rs` | `regression/st0903_incremental.rs` |
| `wave_i1_subtitle_interop.rs` | `regression/subtitle_interop.rs` |

### `io/` — File-I/O helpers, timing smoke, and public-API-surface checks. Pure tst-core.

| old `tests/…` | new `tests/…` |
| --- | --- |
| `io_file_smoke.rs` | `io/io_file_smoke.rs` |
| `public_api_surface.rs` | `io/public_api_surface.rs` |
| `timing_smoke.rs` | `io/timing_smoke.rs` |
