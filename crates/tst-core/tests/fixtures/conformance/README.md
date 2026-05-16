# Codec conformance parameter-set fixtures

Each `.bin` file is one parameter-set body extracted from an official
codec conformance bitstream:

- H.264 / AVC: ITU-T JVT-AVC draft_conformance (AVCv1 and FRExt sub-trees)
- H.265 / HEVC: ITU-T JCT-VC HEVC_v1 and RExt draft_conformance sub-trees
- H.266 / VVC: ITU-T JVET draft_conformance/draft6 sub-tree
- AV1: AOMedia test vectors (storage.googleapis.com/aom-test-data)

The `.bin` is the RBSP body (NAL header stripped, emulation prevention
bytes preserved) for h264/h265/h266, or the OBU payload (OBU header
stripped) for AV1 — matching what the parsers in `tst_core::codec`
accept directly.

The accompanying `.json` sidecar declares the source vector's URL and
sha256 (of the upstream archive, not the stripped fixture) and the
expected parser outcome. See `crates/tst-core/tests/conformance.rs`
for how the test runner consumes them.

## Layout

```
conformance/
  manifest.toml          # hand-curated list of upstream vectors + expected fields
  README.md              # this file
  .gitignore             # excludes _cache/
  _cache/                # gitignored; the strip tool saves downloads here
    .gitkeep
  h264/                  # committed .bin + .json pairs for H.264
  h265/                  # committed .bin + .json pairs for H.265
  h266/                  # committed .bin + .json pairs for H.266
  av1/                   # committed .bin + .json pairs for AV1
```

## Regenerating

The committed `.bin` files are derived from `manifest.toml`. To regenerate:

```
cd ~/Projects/ts-transformer/ts-transformer
cargo run -p tst-core --bin strip_conformance_parameter_sets
```

The tool downloads each upstream archive into `_cache/` (gitignored),
verifies sha256, extracts the named bitstream, scans for parameter-set
NAL/OBU units, and writes `.bin` + `.json` pairs. Reruns are idempotent
when `_cache/` is warm.

## Adding vectors

1. Pick a conformance bitstream from the upstream sources listed above.
2. Download the archive, run `sha256sum`, capture the hash.
3. Add an entry to `manifest.toml` with the URL, sha256, inner filename,
   `kind`, and `expected` fields.
4. Run `cargo run -p tst-core --bin strip_conformance_parameter_sets` to
   produce the `.bin` and `.json` sidecar.
5. Commit the new `.bin` + `.json` files.

## Licensing

The committed fixtures are tiny derived works (parameter-set bytes only,
~50-500 bytes each) from publicly-distributed conformance test sets
maintained by ITU-T / ISO/IEC SCs and AOMedia. The full conformance
vectors are NOT redistributed by this repo — only the parameter-set
bytes are committed, and only for the purpose of testing this codebase's
parsers. See `manifest.toml` for upstream URLs.
