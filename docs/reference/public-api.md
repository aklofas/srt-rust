# Public API Policy

This document codifies the public API conventions for the `ts-transformer`
workspace (`tst-core`, `tst-pipeline`, `tst-srt`, `tst-c`). It applies to
the **pre-1.0 era** — once we reach 1.0, SemVer rules govern; the
conventions here become the *baseline* for what crosses the SemVer
contract boundary.

## Layer model

Three layers of intended-public surface, from most stable to least:

1. **Root re-exports** (`tst_core::Foo`, `tst_pipeline::Foo`, `tst_srt::Foo`)
   — the small set of types and traits a typical caller imports in one
   `use` line. Drift here is high-impact; expect SemVer to take it
   seriously post-1.0.

2. **Module-level public API** (`tst_core::mpegts::demux::Demuxer`,
   `tst_pipeline::mux_sender::MuxSender`, etc.) — the curated, supported
   per-module surface for advanced Rust users. Stability expectations
   match the root re-exports.

3. **`low_level` namespaces** (`tst_core::mpegts::demux::low_level::*`,
   future `tst_core::mpegts::descriptors::low_level::*` if needed) —
   explicitly-named extension points for fuzz harnesses, third-party
   tools, and advanced consumers that need direct access to parser
   internals. **Stability: experimental.** May change between minor
   versions before 1.0; post-1.0 expectations TBD.

Anything not in one of these three categories is **private** and may
change without notice.

## Binding-canonical-workflow rule

Before privatizing a public item, the implementer must:

1. Grep `bindings/c/core/src/` for use sites of the item.
2. Read the `tst-jni` design (outside the published repo at
   `~/Projects/ts-transformer/docs/specs/2026-05-27-tst-jni-design.md`)
   to confirm the planned JVM bindings don't need the item through a
   canonical workflow.
3. Grep `crates/tst-core/fuzz/fuzz_targets/`, `crates/tst-core/tests/`,
   and `examples/` for cross-crate uses.

If any of those checks finds an item being used through a canonical
workflow (i.e., not a hidden field-poking workaround), the item:

- Stays publicly accessible.
- Moves under an explicit `low_level` namespace if the audit confirms
  the use is "advanced consumer territory" rather than "expected
  curated-API consumer territory."

The default bias is **usability over privacy**: keep items reachable for
real consumers, even if it means more public surface. The `low_level`
namespace exists specifically to signal "this is reachable, but you're
opting out of the curated stability contract."

## Module visibility convention

Each module that hosts implementation submodules should follow this
pattern:

```rust
// my_mod/mod.rs
mod helper_a;        // implementation detail
mod helper_b;        // implementation detail
mod types;           // private — re-exported as needed below

pub mod public_sub;  // module is part of the curated surface
pub mod low_level;   // extension points (if applicable)

pub use types::{PublicType, AnotherPublicType};
pub use helper_a::PublicHelperFunction;
```

Submodules that are `pub mod` are the *intentional* public surface.
Submodules that are `mod` are private; if anything inside them needs to
be public, surface it via an explicit `pub use` from `mod.rs`. This
keeps the public surface visible at the top of each `mod.rs`.

## Binding crates: no `cargo public-api` baseline (by design)

Eight Rust library crates carry a committed `public-api.txt` baseline that
CI checks via `cargo public-api` on every push: `rist-sys`, `tst-core`,
`tst-pipeline`, `tst-rist`, `tst-rtp`, `tst-srt`, `tst-tcp`, and `tst-udp`.

The three binding crates — `bindings/c` (tst-c), `bindings/c/core`
(tst-c-core), and `bindings/python` (tst-py) — intentionally carry **no**
`public-api.txt`. Their consumer contract is not their Rust surface:

- **tst-c / tst-c-core.** The Rust surface of these crates is a cdylib/staticlib
  leaf (`pub use tst_c_core::*`) plus `#[no_mangle] extern "C"` glue.
  `cargo public-api` on a cdylib is not meaningful; the real ABI contract is
  the committed cbindgen-generated header `bindings/c/include/tstrans.h`, the
  `TST_ABI_VERSION_MAJOR` / `TST_ABI_VERSION_MINOR` macros it defines, and the
  C-ABI ratchets under `scripts/check/c/` (especially
  `abi-rustdoc-coverage.sh`, `header-conditional-sections.sh`,
  `header-mirror-enum-export.sh`, and `raw-mapper-coverage.sh`).
- **tst-py.** The Rust surface is `#[pymodule]` / `#[pymethods]` PyO3 glue —
  not the Python contract. The Python consumer surface is gated by the
  committed `.pyi` stubs under `bindings/python/python/tstrans/`, the
  `py.typed` marker, the pytest suite, and the Python error-mapping ratchets
  under `scripts/check/python/`.

**Rule:** do not add `cargo public-api` baselines to binding crates unless
they become actual CI release gates. Adding a baseline to a binding crate
that isn't wired into CI creates misleading drift noise without gating
anything.

See `docs/reference/binding-authors.md` for the full C-ABI error-mapping
contract and the Python/JVM/Swift binding-shape conventions.

## Cross-references

- `docs/reference/conventions.md` — naming, constructor verbs, builder rules.
- `docs/reference/binding-authors.md` — JNI / UniFFI / C ABI conventions.
- `docs/reference/architecture.md` — crate graph and high-level pipeline model.
- `docs/project/deferred-features.md` — what's not yet supported and the
  trigger to revisit.

## Examples in this codebase

- `tst_core::mpegts::demux::low_level` (introduced 2026-05-19, plan
  Wave 3.1 Plan A) — re-exports `Reassembler`, `parse_pat`, `parse_pmt`,
  `KlvShape`, `classify_klv`, `walk_descriptors`, and related types so
  fuzz harnesses and advanced consumers reach them without depending on
  private submodule paths.
- `tst_core::mpegts::descriptors` — the canonical home for descriptor
  construction (`registration`, `metadata_klva`, etc.) and parsing
  (`RawDescriptor`, `walk_descriptors`, `find_descriptor_tag`, etc.).
