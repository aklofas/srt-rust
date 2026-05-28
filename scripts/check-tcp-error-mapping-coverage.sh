#!/usr/bin/env bash
# 28th bash ratchet (Plan A5a Wave B).
# Verifies every TcpErrorKind variant from tst-tcp has a mapping arm in
# crates/tst-c/src/error.rs's tcp_error_to_code function.

set -euo pipefail

ERR_FILE="crates/tst-c/src/error.rs"
TCP_ERR="crates/tst-tcp/src/error.rs"

if [[ ! -f "$ERR_FILE" ]]; then
    echo "FAIL: $ERR_FILE not found"
    exit 1
fi
if [[ ! -f "$TCP_ERR" ]]; then
    echo "FAIL: $TCP_ERR not found"
    exit 1
fi

# Extract TcpErrorKind variants from tst-tcp.
# The enum is declared like `pub enum TcpErrorKind { Url = 1, Io = 2, ... }`.
# We grep for variant lines (CamelCase identifiers followed by `=` or `,`).
TCP_VARIANTS=$(awk '/pub enum TcpErrorKind/,/^}/' "$TCP_ERR" | \
    grep -oE '^\s+([A-Z][A-Za-z0-9]+)' | \
    sed 's/^[[:space:]]*//' | sort -u)

# Extract variants matched inside tcp_error_to_code.
MAPPED=$(awk '/fn tcp_error_to_code/,/^}/' "$ERR_FILE" | \
    grep -oE 'TcpErrorKind::([A-Z][A-Za-z0-9]+)' | \
    sed 's/TcpErrorKind:://' | sort -u)

MISSING=$(comm -23 <(echo "$TCP_VARIANTS") <(echo "$MAPPED") | grep -v '^$' || true)
if [[ -n "$MISSING" ]]; then
    echo "FAIL: TcpErrorKind variants missing C error code mapping in tcp_error_to_code:"
    echo "$MISSING"
    exit 1
fi

# Also forbid wildcard arms unless TcpErrorKind is #[non_exhaustive].
NON_EXHAUSTIVE=$(grep -c '#\[non_exhaustive\]' "$TCP_ERR" || true)
HAS_WILDCARD=$(awk '/fn tcp_error_to_code/,/^}/' "$ERR_FILE" | grep -cE '_\s*=>' || true)

if [[ "$NON_EXHAUSTIVE" -eq 0 && "$HAS_WILDCARD" -gt 0 ]]; then
    echo "FAIL: tcp_error_to_code uses wildcard arm but TcpErrorKind is not #[non_exhaustive]"
    echo "Either remove the wildcard or mark the enum non_exhaustive."
    exit 1
fi

echo "PASS: tcp-error-mapping-coverage"
