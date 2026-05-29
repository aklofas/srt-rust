# Test coverage control plane (advisory)

This directory holds **advisory** manifests that describe the test suite — what
runs where, what's intentionally skipped, what coverage is intended, and where
every committed fixture came from. They are **inventory / review aids**, not
coverage gates: `scripts/check-test-manifests.sh` enforces only that they are
*well-formed and reference real, in-tree files* — never that coverage is
"enough". Real enforcement (a surface manifest, enforced coverage) is deferred
to a later milestone.

## The tier model

Every test (or test job) maps to one of these tiers:

| Tier | Meaning | Where it runs | Gating? |
| --- | --- | --- | --- |
| **A** | Always-on, hermetic, fast. The default suite. | PR CI (`build (linux-x86_64)`, `build (linux-aarch64)`), both feature modes | **Yes** — a red here blocks merge |
| **A-soft** | Always-on but on a soft platform, phasing in. | PR CI macOS / Windows jobs (`continue-on-error: true`) | No (soft red) — promoted to A on a dated schedule |
| **B** | Needs external tools (ffmpeg / tsduck / vlc) or the long soak. | Local `scripts/release-validation.sh` before tagging | No (local, pre-release) |
| **C** | Manual / diagnostic. Run explicitly by a maintainer. | `#[ignore]`d; run with `-- --ignored` on demand | No |
| **D** | Deferred / advisory. Documented intent, not yet implemented. | Nowhere yet | No |

Notes:
- The Rust integration tests under `crates/*/tests/<domain>/` and the unit
  tests are **Tier A**. The fuzz compile-check is Tier A; long fuzz runs are
  Tier B (see `fuzz-targets.toml` + `scripts/release-validation.sh`).
- ffmpeg/tsduck round-trip tests and the 1-hour soak are **Tier B**.
- Tests marked `#[ignore]` for an external decoder, a real encoder, or a
  diagnostic dump are **Tier C** (catalogued in `skip-ledger.toml`).

## tst-integration CI modes

The cross-binding scenario harness crate (`crates/tst-integration`, when present)
runs in pinned modes so it never slows the default workspace run or pulls in
built bindings it doesn't need:

- **default features (Rust-only)** — the Rust scenario adapter; Tier A.
- **`c` feature** — the C adapter; needs `libtstrans` built first; explicit CI job.
- **`python` feature** — the Python adapter; needs the `tstrans` wheel; explicit CI job.

## Files here

- **`fuzz-targets.toml`** — generated inventory of every libFuzzer target;
  `scripts/gen-fuzz-targets.sh --check` fails CI if it drifts from the tree.
- **`skip-ledger.toml`** — every intentional skip / `#[ignore]`, with a class
  and (for placeholders / blocked bugs) an expiry.
- **`stream-matrix.toml`** — advisory coverage intent across codec / audio /
  subtitle / KLV / program / descriptor axes; marks known gaps.
- **`fixture-manifest.toml`** — provenance for every committed fixture group:
  origin (synthetic / public / derived), generator, and a no-private-corpus flag.
- **`TEST_CORPUS.md`** — notes on the (gitignored, local-only) real-world corpus.
