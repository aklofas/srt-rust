# OSS-Fuzz local verification log

**Run:** 2026-05-15
**Reviewer:** andrew.klofas@gmail.com

## Build method

Built with local source mount (required because Tasks 1-10 commits are not yet pushed to
GitHub — the Dockerfile clones from GitHub, so a `--mount_path` or `source_path` argument
is needed for pre-push local verification):

```
python3 infra/helper.py build_fuzzers --sanitizer address --clean \
  ts-transformer /home/aklofas/Projects/ts-transformer/ts-transformer
```

Build reported: `INFO: shipped 39 fuzz drivers to $OUT`

## check_build

```
INFO: performing bad build checks for demux_psi
INFO: performing bad build checks for demux_pes_reassembly
INFO: performing bad build checks for audio_frame_iter
INFO: performing bad build checks for klv_st0903_decode
INFO: performing bad build checks for url_parse
INFO: performing bad build checks for klv_st0601_decode
INFO: performing bad build checks for mux_pull
INFO: performing bad build checks for mux_push_klv
INFO: performing bad build checks for mpegts_au_cell_read
INFO: performing bad build checks for klv_iter
INFO: performing bad build checks for mux_push_video
INFO: performing bad build checks for klv_st0102_decode
INFO: performing bad build checks for parse_parameter_sets
INFO: performing bad build checks for demux_feed
INFO: performing bad build checks for ts_parser
INFO: performing bad build checks for parse_av1_sequence_header
INFO:__main__:Check build passed.
```

All 16 targets check_build = PASS.

## run_fuzzer smoke (1k iters each)

14 of 16 targets ran 1000 libFuzzer iterations without crash. 2 targets surfaced issues:

- **demux_psi**: OOB slice at `psi.rs:90` — real library bug. See "Known issues" below.
- **klv_st0903_decode**: round-trip assertion failure — harness logic mismatch with plan #46 encode semantics, NOT a library bug. See "Known issues" below.

`demux_feed` ran 10,000 iterations without crash — satisfies spec acceptance criterion #4:

```
#2	  INITED cov: 141 ft: 142 corp: 1/1710b exec/s: 0 rss: 31Mb
#10000	DONE   cov: 279 ft: 596 corp: 65/66Kb lim: 1710 exec/s: 0 rss: 50Mb
```

## Artifact inventory in $OUT/

Built with local source. Counts confirmed by Docker container inspection:

- 16 fuzz driver binaries (+ llvm-symbolizer = 17 executables)
- 14 `*_seed_corpus.zip` files
  - fixture-derived: demux_feed, demux_pes_reassembly, demux_psi, klv_st0601_decode, ts_parser
  - synthetic: audio_frame_iter, klv_iter, mpegts_au_cell_read, mux_pull, mux_push_klv,
    mux_push_video, parse_av1_sequence_header, parse_parameter_sets, url_parse
  - intentionally absent: klv_st0102_decode, klv_st0903_decode (no seeds committed)
- 4 `*.options` files: demux_feed, demux_pes_reassembly, demux_psi, ts_parser
- 4 `*.dict` files: klv_iter, klv_st0102_decode, klv_st0601_decode, klv_st0903_decode
- Plus llvm-symbolizer (OSS-Fuzz infrastructure helper)

Total: 39 files in $OUT.

## Known issues — to fix before opening the OSS-Fuzz submission PR

The 1k-iteration smoke pass surfaced two issues. Neither is a Task 11 blocker for shipping the OSS-Fuzz infrastructure, but both should be resolved in a follow-up plan before opening the PR to `google/oss-fuzz` (otherwise the OSS-Fuzz fleet will surface them on day 1 as its first bug reports).

### 1. `demux_psi`: out-of-bounds slice in `parse_pat` (REAL library bug)

- **Location:** `crates/tst-core/src/mpegts/demux/psi.rs:90`
- **Cause:** `section[..total_len - 4]` is evaluated without first checking `total_len >= 4`. When `section_length` is small (0/1/2 → `total_len` 3/4/5), the subtraction underflows or yields a degenerate slice.
- **Fix shape:** add a guard `if total_len < 4 { return Err(PsiParseError::TruncatedSection); }` before the CRC slice extraction. Verify behavior aligns with the existing `PsiParseError` variants.

### 2. `klv_st0903_decode`: harness round-trip assertion failure (NOT a library bug)

- **Location:** `crates/tst-core/fuzz/fuzz_targets/klv_st0903_decode.rs:28`
- **Cause:** The harness does an encode→decode→encode→decode round-trip and asserts `decoded_a == decoded_b`. Per plan #46, `klv::st0903::encode` deliberately drops Tag 1 (checksum) and lets the muxer handle it externally. When the original input contains a checksum byte, decode populates `checksum: Some(N)`; the next encode strips it; the second decode sees no checksum. The two `decoded` values therefore differ in their `checksum` field.
- **Fix shape:** EITHER (a) exclude `checksum` from the round-trip equality check in the harness, OR (b) use `encode_standalone` (which DOES include Tag 1, per plan #46's added entry point) for the re-encode step. Option (b) is more semantically faithful to "round-trip parity."
- **Minimal reproducer:** 4 bytes `\x01\x02\x00\x11`.

Both fixes are tiny (a single guard for #1, a 1-2-line harness adjustment for #2). They are out of scope for plan #53 (OSS-Fuzz infrastructure onboarding) and will be addressed as a tiny follow-up plan #54 before the user opens the actual PR to google/oss-fuzz.
