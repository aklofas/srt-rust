#!/usr/bin/env bash
# 29th bash ratchet (Plan A5a Wave C).
# Verifies every HlsErrorKind variant from tst-tcp/hls has a mapping arm in
# crates/tst-c/src/error.rs's hls_error_to_code function.

set -euo pipefail

ERR_FILE="crates/tst-c/src/error.rs"
HLS_ERR="crates/tst-tcp/src/hls/error.rs"

if [[ ! -f "$ERR_FILE" ]]; then
    echo "FAIL: $ERR_FILE not found"
    exit 1
fi
if [[ ! -f "$HLS_ERR" ]]; then
    echo "FAIL: $HLS_ERR not found"
    exit 1
fi

# Extract HlsErrorKind variants from tst-tcp/hls.
# The enum is declared like `pub enum HlsErrorKind { Url = 1, Io = 2, ... }`.
# We grep for variant lines (CamelCase identifiers followed by `=` or `,`).
HLS_VARIANTS=$(awk '/pub enum HlsErrorKind/,/^}/' "$HLS_ERR" | \
    grep -oE '^\s+([A-Z][A-Za-z0-9]+)' | \
    sed 's/^[[:space:]]*//' | sort -u)

# Extract variants matched inside hls_error_to_code.
MAPPED=$(awk '/fn hls_error_to_code/,/^}/' "$ERR_FILE" | \
    grep -oE 'HlsErrorKind::([A-Z][A-Za-z0-9]+)' | \
    sed 's/HlsErrorKind:://' | sort -u)

MISSING=$(comm -23 <(echo "$HLS_VARIANTS") <(echo "$MAPPED") | grep -v '^$' || true)
if [[ -n "$MISSING" ]]; then
    echo "FAIL: HlsErrorKind variants missing C error code mapping in hls_error_to_code:"
    echo "$MISSING"
    exit 1
fi

# Also forbid wildcard arms unless HlsErrorKind is #[non_exhaustive].
NON_EXHAUSTIVE=$(grep -c '#\[non_exhaustive\]' "$HLS_ERR" || true)
HAS_WILDCARD=$(awk '/fn hls_error_to_code/,/^}/' "$ERR_FILE" | grep -cE '_\s*=>' || true)

if [[ "$NON_EXHAUSTIVE" -eq 0 && "$HAS_WILDCARD" -gt 0 ]]; then
    echo "FAIL: hls_error_to_code uses wildcard arm but HlsErrorKind is not #[non_exhaustive]"
    echo "Either remove the wildcard or mark the enum non_exhaustive."
    exit 1
fi

echo "PASS: hls-error-mapping-coverage"
