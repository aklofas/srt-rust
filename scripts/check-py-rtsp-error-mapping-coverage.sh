#!/usr/bin/env bash
# 24th bash ratchet (tst-py Phase 4 Stage 2 Bootstrap): every variant in
# tstrans.exceptions.RtspErrorKind must have at least one `make_rtsp_error`
# call site in crates/tst-py/src/ — symmetric with the existing
# check-py-codec-error-mapping-coverage.sh.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PY_EXC_FILE="$ROOT/crates/tst-py/python/tstrans/exceptions.py"
SRC_DIR="$ROOT/crates/tst-py/src"

# Expected variants: extract every line inside the `class RtspErrorKind`
# block that looks like `NAME = N`. Each is a SHOUTY_SNAKE identifier.
expected=$(awk '
  /^class RtspErrorKind/ { in_block = 1; next }
  in_block && /^class / && !/^class RtspErrorKind/ { in_block = 0 }
  in_block && /^    [A-Z_][A-Z0-9_]* = [0-9]+/ {
    sub(/^    /, "")
    sub(/ .*$/, "")
    print
  }
' "$PY_EXC_FILE" | sort -u)

# Find every `make_rtsp_error(<python>, "KIND", ...)` call site and
# `_raise_rtsp_error_for_test` reachability (latter forwards an arg to
# make_rtsp_error so kinds passed via test fixtures count too).
used=$(grep -roh -E 'make_rtsp_error\([^,]+,\s*"[A-Z_][A-Z0-9_]*"' "$SRC_DIR" \
    | grep -oE '"[A-Z_][A-Z0-9_]*"' \
    | tr -d '"' \
    | sort -u)

missing=$(comm -23 <(echo "$expected") <(echo "$used"))

if [[ -n "$missing" ]]; then
    echo "FAIL: RtspErrorKind variants without make_rtsp_error call site:" >&2
    while read -r v; do echo "  - $v" >&2; done <<< "$missing"
    echo "Add a make_rtsp_error(py, \"<VARIANT>\", ...) somewhere in crates/tst-py/src/" >&2
    exit 1
fi

echo "OK: all $(echo "$expected" | wc -l | tr -d ' ') RtspErrorKind variants mapped"
