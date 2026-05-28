#!/usr/bin/env bash
# 31st bash ratchet (Plan A5a Wave D).
# Verifies every RistErrorKind variant from tst-rist has a mapping arm in
# crates/tst-c/src/error.rs's rist_error_to_code function.

set -euo pipefail

ERR_FILE="crates/tst-c/src/error.rs"
RIST_ERR="crates/tst-rist/src/error.rs"

if [[ ! -f "$ERR_FILE" ]]; then
    echo "FAIL: $ERR_FILE not found"
    exit 1
fi
if [[ ! -f "$RIST_ERR" ]]; then
    echo "FAIL: $RIST_ERR not found"
    exit 1
fi

# Extract RistErrorKind variants from tst-rist.
# The enum is declared like `pub enum RistErrorKind { Url = 1, Ffi = 2, ... }`.
# We grep for variant lines (CamelCase identifiers followed by `=` or `,`).
RIST_VARIANTS=$(awk '/pub enum RistErrorKind/,/^}/' "$RIST_ERR" | \
    grep -oE '^\s+([A-Z][A-Za-z0-9]+)' | \
    sed 's/^[[:space:]]*//' | sort -u)

# Extract variants matched inside rist_error_to_code.
MAPPED=$(awk '/fn rist_error_to_code/,/^}/' "$ERR_FILE" | \
    grep -oE 'RistErrorKind::([A-Z][A-Za-z0-9]+)' | \
    sed 's/RistErrorKind:://' | sort -u)

MISSING=$(comm -23 <(echo "$RIST_VARIANTS") <(echo "$MAPPED") | grep -v '^$' || true)
if [[ -n "$MISSING" ]]; then
    echo "FAIL: RistErrorKind variants missing C error code mapping in rist_error_to_code:"
    echo "$MISSING"
    exit 1
fi

# Also forbid wildcard arms unless RistErrorKind is #[non_exhaustive].
NON_EXHAUSTIVE=$(grep -c '#\[non_exhaustive\]' "$RIST_ERR" || true)
HAS_WILDCARD=$(awk '/fn rist_error_to_code/,/^}/' "$ERR_FILE" | grep -cE '_\s*=>' || true)

if [[ "$NON_EXHAUSTIVE" -eq 0 && "$HAS_WILDCARD" -gt 0 ]]; then
    echo "FAIL: rist_error_to_code uses wildcard arm but RistErrorKind is not #[non_exhaustive]"
    echo "Either remove the wildcard or mark the enum non_exhaustive."
    exit 1
fi

echo "PASS: rist-error-mapping-coverage"
