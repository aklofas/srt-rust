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

14 of 16 targets ran 1000 libFuzzer iterations without crash. 2 targets found real panics:

- **demux_psi**: out-of-bounds slice index in `psi.rs:90` (`parse_pat`). ASAN ABRT.
- **klv_st0903_decode**: panic in VMTI decode path. ASAN ABRT.

Both panics are Rust `index-out-of-bounds` panics (not memory corruption). These are real
bugs for the bug-triage path. They are NOT blocking OSS-Fuzz submission — OSS-Fuzz would
surface them on day 1 via its continuous fuzzing. See DONE_WITH_CONCERNS note below.

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

## Known bugs found during verification

These should be fixed before or shortly after OSS-Fuzz submission:

1. **demux_psi panic** — `parse_pat` at `crates/tst-core/src/mpegts/demux/psi.rs:90`:
   slice index out of bounds. Crash input: `crash-c98fce95d337b61d86b54682b09fe9d92b231614`.

2. **klv_st0903_decode panic** — VMTI decode path. Crash input:
   `crash-3ca500c27e18065531c84e9f9e26329ac04c337d`.
   Minimal reproducer: `\x01\x02\x00\x11` (4 bytes, Base64: `AQIAEQ==`).
