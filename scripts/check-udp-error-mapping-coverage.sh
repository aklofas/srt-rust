#!/usr/bin/env bash
# 27th bash ratchet (Plan A5a Wave A).
# Verifies every UdpErrorKind variant from tst-udp has a mapping arm in
# crates/tst-c/src/error.rs's udp_error_to_code function.

set -euo pipefail

ERR_FILE="crates/tst-c/src/error.rs"
UDP_ERR="crates/tst-udp/src/error.rs"

if [[ ! -f "$ERR_FILE" ]]; then
    echo "FAIL: $ERR_FILE not found"
    exit 1
fi
if [[ ! -f "$UDP_ERR" ]]; then
    echo "FAIL: $UDP_ERR not found"
    exit 1
fi

# Extract UdpErrorKind variants from tst-udp.
# The enum is declared like `pub enum UdpErrorKind { Url = 1, HostNotLiteral = 2, ... }`.
# We grep for variant lines (CamelCase identifiers followed by `=` or `,`).
UDP_VARIANTS=$(awk '/pub enum UdpErrorKind/,/^}/' "$UDP_ERR" | \
    grep -oE '^\s+([A-Z][A-Za-z0-9]+)' | \
    sed 's/^[[:space:]]*//' | sort -u)

# Extract variants matched inside udp_error_to_code.
MAPPED=$(awk '/fn udp_error_to_code/,/^}/' "$ERR_FILE" | \
    grep -oE 'UdpErrorKind::([A-Z][A-Za-z0-9]+)' | \
    sed 's/UdpErrorKind:://' | sort -u)

MISSING=$(comm -23 <(echo "$UDP_VARIANTS") <(echo "$MAPPED") | grep -v '^$' || true)
if [[ -n "$MISSING" ]]; then
    echo "FAIL: UdpErrorKind variants missing C error code mapping in udp_error_to_code:"
    echo "$MISSING"
    exit 1
fi

# Also forbid wildcard arms unless UdpErrorKind is #[non_exhaustive].
NON_EXHAUSTIVE=$(grep -c '#\[non_exhaustive\]' "$UDP_ERR" || true)
HAS_WILDCARD=$(awk '/fn udp_error_to_code/,/^}/' "$ERR_FILE" | grep -cE '_\s*=>' || true)

if [[ "$NON_EXHAUSTIVE" -eq 0 && "$HAS_WILDCARD" -gt 0 ]]; then
    echo "FAIL: udp_error_to_code uses wildcard arm but UdpErrorKind is not #[non_exhaustive]"
    echo "Either remove the wildcard or mark the enum non_exhaustive."
    exit 1
fi

echo "PASS: udp-error-mapping-coverage"
