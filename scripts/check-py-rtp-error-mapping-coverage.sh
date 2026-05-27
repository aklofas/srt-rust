#!/usr/bin/env bash
# 25th bash ratchet (tst-py Phase 4 Stage 2 Bootstrap): every variant in
# tstrans.exceptions.RtpErrorKind must have at least one `make_rtp_error`
# call site in crates/tst-py/src/.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PY_EXC_FILE="$ROOT/crates/tst-py/python/tstrans/exceptions.py"
SRC_DIR="$ROOT/crates/tst-py/src"

expected=$(awk '
  /^class RtpErrorKind/ { in_block = 1; next }
  in_block && /^class / && !/^class RtpErrorKind/ { in_block = 0 }
  in_block && /^    [A-Z_][A-Z0-9_]* = [0-9]+/ {
    sub(/^    /, "")
    sub(/ .*$/, "")
    print
  }
' "$PY_EXC_FILE" | sort -u)

used=$(grep -roh -E 'make_rtp_error\([^,]+,\s*"[A-Z_][A-Z0-9_]*"' "$SRC_DIR" \
    | grep -oE '"[A-Z_][A-Z0-9_]*"' \
    | tr -d '"' \
    | sort -u)

missing=$(comm -23 <(echo "$expected") <(echo "$used"))

if [[ -n "$missing" ]]; then
    echo "FAIL: RtpErrorKind variants without make_rtp_error call site:" >&2
    while read -r v; do echo "  - $v" >&2; done <<< "$missing"
    echo "Add a make_rtp_error(py, \"<VARIANT>\", ...) somewhere in crates/tst-py/src/" >&2
    exit 1
fi

echo "OK: all $(echo "$expected" | wc -l | tr -d ' ') RtpErrorKind variants mapped"
