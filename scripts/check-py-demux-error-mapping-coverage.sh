#!/usr/bin/env bash
# Plan #96 Wave G (Finding 12): every DemuxError variant in tst-core must
# appear explicitly in tst-py/src/mpegts.rs `demux_error_to_pyerr` mapper.
#
# `DemuxError` is `#[non_exhaustive]` and the mapper carries a wildcard
# fallback to "INTERNAL" for forward-compat. This ratchet ensures every
# CURRENTLY-DEFINED variant has its own explicit arm BEFORE the wildcard,
# so future Rust additions cannot silently route to INTERNAL when a more
# specific Python `DemuxErrorKind` already fits.
#
# Companion to `check-py-codec-error-mapping-coverage.sh` (12th ratchet);
# this is the analogous coverage for the demuxer error surface.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
RUST_FILE="$ROOT/crates/tst-core/src/error.rs"
PY_FILE="$ROOT/bindings/python/src/mpegts.rs"

# Extract every variant name from `pub enum DemuxError { ... }` block.
# Portable read-into-array pattern (bash 3.2+) per
# `feedback_bash_ratchets_macos_portability.md` — no `mapfile`/`readarray`.
variants=()
while IFS= read -r v; do
    variants+=("$v")
done < <(awk '
    /^pub enum DemuxError/ { in_block = 1; next }
    in_block && /^}/ { in_block = 0 }
    in_block && /^    [A-Z][A-Za-z0-9]*[ ({,]/ {
        sub(/^    /, "")
        sub(/[ {(,].*$/, "")
        print
    }
' "$RUST_FILE")

if [ "${#variants[@]}" -eq 0 ]; then
    echo "FAIL: extracted 0 DemuxError variants — awk pattern may have drifted from $RUST_FILE" >&2
    exit 1
fi

missing=()
for v in "${variants[@]}"; do
    if ! grep -qE "DemuxError::${v}\b" "$PY_FILE"; then
        missing+=("$v")
    fi
done

if [ "${#missing[@]}" -ne 0 ]; then
    echo "ERROR: DemuxError variants missing explicit arm in $PY_FILE:" >&2
    for v in "${missing[@]}"; do echo "  - $v" >&2; done
    echo "" >&2
    echo "Add an explicit \`DemuxError::<Variant> =>\` arm in" >&2
    echo "\`demux_error_to_pyerr\` before the wildcard fallback so the new" >&2
    echo "variant maps to a specific DemuxErrorKind instead of silently" >&2
    echo "routing to INTERNAL." >&2
    exit 1
fi

echo "OK: all ${#variants[@]} DemuxError variants mapped explicitly in demux_error_to_pyerr"
