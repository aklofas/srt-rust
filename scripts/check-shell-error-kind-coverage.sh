#!/usr/bin/env bash
# Verify every ShellErrorKind variant is matched explicitly in
# tst_error_from_kind (crates/tst-c/src/error.rs) BEFORE the function's
# wildcard arm.
#
# ShellErrorKind is #[non_exhaustive], requiring tst-c match expressions
# to include a wildcard arm. Rust's exhaustiveness checker cannot tell
# us when a new ShellErrorKind variant has been added because the
# wildcard always matches. This ratchet fills that gap.
#
# Replaces the variant-level scripts/check-tst-c-error-coverage.sh
# (deleted in Plan A Task 10).

set -euo pipefail

KIND_FILE="crates/tst-pipeline/src/shell_error.rs"
TST_C_ERROR="crates/tst-c/src/error.rs"

extract_function_body_before_wildcard() {
    local file="$1"
    local fn_name="$2"
    awk -v fn="fn $fn_name" '
        BEGIN { inside = 0; depth = 0; started = 0 }
        $0 ~ fn { inside = 1 }
        inside && started == 0 {
            for (i = 1; i <= length($0); i++) {
                c = substr($0, i, 1)
                if (c == "{") { depth++; started = 1 }
            }
            print
            next
        }
        inside && started {
            trimmed = $0
            sub(/^[ \t]+/, "", trimmed)
            if (trimmed ~ /^_[ \t]*=>/) { exit }
            print
            for (i = 1; i <= length($0); i++) {
                c = substr($0, i, 1)
                if (c == "{") depth++
                else if (c == "}") { depth--; if (depth == 0) exit }
            }
        }
    ' "$file"
}

# Extract ShellErrorKind variants.
variants=$(awk '/^pub enum ShellErrorKind \{/,/^}/' "$KIND_FILE" \
    | grep -oP '^\s+\K[A-Z][A-Za-z0-9]+(?=\s*,)')

if [[ -z "$variants" ]]; then
    echo "FAIL: no ShellErrorKind variants found in $KIND_FILE"
    exit 1
fi

fn_body=$(extract_function_body_before_wildcard "$TST_C_ERROR" "tst_error_from_kind")

if [[ -z "$fn_body" ]]; then
    echo "FAIL: could not locate fn tst_error_from_kind in $TST_C_ERROR"
    exit 1
fi

missing=0
total=0
for v in $variants; do
    total=$((total + 1))
    if ! echo "$fn_body" | grep -q "ShellErrorKind::$v\b"; then
        echo "MISSING: ShellErrorKind::$v not handled in tst_error_from_kind (before wildcard)"
        missing=$((missing + 1))
    fi
done

if [[ $missing -gt 0 ]]; then
    echo ""
    echo "FAIL: $missing ShellErrorKind variant(s) not handled at the C ABI"
    echo "Add an explicit arm to tst_error_from_kind in $TST_C_ERROR"
    echo "BEFORE the wildcard _ => arm."
    exit 1
fi

echo "OK: all $total ShellErrorKind variant(s) handled in tst_error_from_kind"
