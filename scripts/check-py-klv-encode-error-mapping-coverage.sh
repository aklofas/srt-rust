#!/usr/bin/env bash
# Plan #96 Wave G (Finding 12): every KlvEncodeError variant in tst-core
# must appear explicitly in tst-py/src/errors.rs
# `klv_encode_error_to_pyerr` mapper.
#
# `KlvEncodeError` is `#[non_exhaustive]` and the mapper carries a
# wildcard fallback to "BUFFER_TOO_SMALL" for forward-compat. This
# ratchet ensures every CURRENTLY-DEFINED variant has its own explicit
# arm BEFORE the wildcard, so future Rust additions cannot silently
# route to BUFFER_TOO_SMALL when a more specific Python
# `KlvEncodeErrorKind` already fits.
#
# The mapper aliases `KlvEncodeError` as `RustE` locally; the variant
# check accepts either spelling.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
RUST_FILE="$ROOT/crates/tst-core/src/error.rs"
PY_FILE="$ROOT/crates/tst-py/src/errors.rs"

# Extract every variant name from `pub enum KlvEncodeError { ... }` block.
# Portable read-into-array pattern (bash 3.2+) per
# `feedback_bash_ratchets_macos_portability.md` — no `mapfile`/`readarray`.
variants=()
while IFS= read -r v; do
    variants+=("$v")
done < <(awk '
    /^pub enum KlvEncodeError/ { in_block = 1; next }
    in_block && /^}/ { in_block = 0 }
    in_block && /^    [A-Z][A-Za-z0-9]*[ ({,]/ {
        sub(/^    /, "")
        sub(/[ {(,].*$/, "")
        print
    }
' "$RUST_FILE")

if [ "${#variants[@]}" -eq 0 ]; then
    echo "FAIL: extracted 0 KlvEncodeError variants — awk pattern may have drifted from $RUST_FILE" >&2
    exit 1
fi

missing=()
for v in "${variants[@]}"; do
    # Mapper file may reference variants via the full type name or the
    # `RustE` local alias defined in `klv_encode_error_to_pyerr`.
    if ! grep -qE "(KlvEncodeError|RustE)::$v\b" "$PY_FILE"; then
        missing+=("$v")
    fi
done

if [ "${#missing[@]}" -ne 0 ]; then
    echo "ERROR: KlvEncodeError variants missing explicit arm in $PY_FILE:" >&2
    for v in "${missing[@]}"; do echo "  - $v" >&2; done
    echo "" >&2
    echo "Add an explicit \`RustE::<Variant> =>\` arm in" >&2
    echo "\`klv_encode_error_to_pyerr\` before the wildcard fallback so" >&2
    echo "the new variant maps to a specific KlvEncodeErrorKind instead" >&2
    echo "of silently routing to BUFFER_TOO_SMALL." >&2
    exit 1
fi

echo "OK: all ${#variants[@]} KlvEncodeError variants mapped explicitly in klv_encode_error_to_pyerr"
