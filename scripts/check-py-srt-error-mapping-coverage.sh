#!/usr/bin/env bash
# 26th bash ratchet (tst-py Phase 8 Wave A T4): every variant in
# tstrans.exceptions.SrtErrorKind must have at least one
# `make_srt_error(py, "<KIND>", ...)` call site under crates/tst-py/src/.
#
# Mirrors check-py-rtp-error-mapping-coverage.sh. Line-based grep — the
# kind literal MUST be on the same line as the open-paren; multi-line
# `make_srt_error(py,\n "KIND"` wraps will NOT match (see
# [[feedback-bash-ratchet-line-based-grep]]).

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PY_EXC_FILE="$ROOT/crates/tst-py/python/tstrans/exceptions.py"
SRC_DIR="$ROOT/crates/tst-py/src"

expected=$(awk '
  /^class SrtErrorKind/ { in_block = 1; next }
  in_block && /^class / && !/^class SrtErrorKind/ { in_block = 0 }
  in_block && /^    [A-Z_][A-Z0-9_]* = [0-9]+/ {
    sub(/^    /, "")
    sub(/ .*$/, "")
    print
  }
' "$PY_EXC_FILE" | sort -u)

# `grep -v '^\s*//'` filters Rust line/doc comments so an example
# `make_srt_error(py, "KIND", ...)` in a module-level rustdoc doesn't
# get treated as a real call site.
used=$(grep -rh -E 'make_srt_error\([^,]+,\s*"[A-Z_][A-Z0-9_]*"' "$SRC_DIR" \
    | grep -v -E '^\s*//' \
    | grep -oE 'make_srt_error\([^,]+,\s*"[A-Z_][A-Z0-9_]*"' \
    | grep -oE '"[A-Z_][A-Z0-9_]*"' \
    | tr -d '"' \
    | sort -u)

missing=$(comm -23 <(echo "$expected") <(echo "$used"))

if [[ -n "$missing" ]]; then
    echo "FAIL: SrtErrorKind variants without make_srt_error call site:" >&2
    while read -r v; do echo "  - $v" >&2; done <<< "$missing"
    echo "Add a make_srt_error(py, \"<VARIANT>\", ...) somewhere in crates/tst-py/src/" >&2
    exit 1
fi

unknown=$(comm -13 <(echo "$expected") <(echo "$used"))
if [[ -n "$unknown" ]]; then
    echo "FAIL: make_srt_error call sites with unrecognized kind:" >&2
    while read -r v; do echo "  - $v" >&2; done <<< "$unknown"
    echo "Either add the variant to tstrans.exceptions.SrtErrorKind or fix the call site." >&2
    exit 1
fi

echo "OK: all $(echo "$expected" | wc -l | tr -d ' ') SrtErrorKind variants mapped"
