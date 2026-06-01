#!/usr/bin/env bash
# Verify every variant of MuxError + TransportError is matched explicitly
# in the raw record_*_error mappers in crates/tst-c-core/src/error.rs BEFORE
# the mapper's wildcard arm.
#
# These raw mappers exist because the standalone-muxer path (no shell)
# and the open/connect/listen helper paths (raw TransportError surfaced
# before any shell wraps it) bypass the shell-layer ShellErrorKind
# routing that check-shell-error-kind-coverage.sh /
# check-pipeline-kind-classification.sh guard.
#
# Without this ratchet, a new MuxError or TransportError variant could
# slip through to the wildcard arm at runtime, surfacing as
# TST_E_INVALID_CONFIG / TST_E_TRANSPORT with no actionable diagnostic
# beyond the Debug-formatted variant name — the exact regression the
# Wave 1.3 ratchet (plan #70) was designed to prevent.

set -euo pipefail

TST_C_ERROR="crates/tst-c-core/src/error.rs"

# Each row: enum-file | enum-short-name | mapper-fn-name | excluded-variants (space-separated, optional)
#
# excluded-variants: variants that are deliberately not in this mapper
# because the raw path cannot encounter them. Currently:
#   TransportError::ExplicitClose — only emitted by ManagedRecvTransport
#   from the shell layer (Plan B). Raw connect/listen helpers never
#   construct it. If raw paths ever start emitting it, drop the
#   exclusion + add an explicit arm.
MAPPERS=(
    "crates/tst-core/src/error.rs|MuxError|record_mux_error|"
    "crates/tst-core/src/transport.rs|TransportError|record_transport_error|ExplicitClose"
)

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

extract_enum_variants() {
    local file="$1"
    local enum_short="$2"
    awk -v name="pub enum $enum_short" '
        BEGIN { inside = 0; depth = 0 }
        $0 ~ name { inside = 1 }
        inside {
            for (i = 1; i <= length($0); i++) {
                c = substr($0, i, 1)
                if (c == "{") depth++
                else if (c == "}") { depth--; if (depth == 0) { print; exit } }
            }
            print
        }
    ' "$file" | grep -oP '^\s*\K[A-Z][A-Za-z0-9]+(?=\s*[,({])'
}

missing=0
total=0
for spec in "${MAPPERS[@]}"; do
    enum_file="${spec%%|*}"
    rest="${spec#*|}"
    enum_short="${rest%%|*}"
    rest2="${rest#*|}"
    fn_name="${rest2%%|*}"
    excluded="${rest2#*|}"

    variants=$(extract_enum_variants "$enum_file" "$enum_short")
    if [[ -z "$variants" ]]; then
        echo "WARN: zero variants for $enum_short in $enum_file"
        continue
    fi

    fn_body=$(extract_function_body_before_wildcard "$TST_C_ERROR" "$fn_name")
    if [[ -z "$fn_body" ]]; then
        echo "FAIL: could not locate fn $fn_name in $TST_C_ERROR"
        exit 1
    fi

    n=0
    for v in $variants; do
        # Skip variants on the excluded list.
        skip=0
        for ex in $excluded; do
            if [[ "$v" == "$ex" ]]; then
                skip=1
                break
            fi
        done
        if [[ $skip -eq 1 ]]; then
            continue
        fi

        n=$((n + 1))
        total=$((total + 1))
        if ! echo "$fn_body" | grep -q "$enum_short::$v\b"; then
            echo "MISSING: $enum_short::$v not handled in $fn_name (before wildcard)"
            missing=$((missing + 1))
        fi
    done

    echo "checked $enum_short ($n variants, excluded: ${excluded:-<none>}) -> $fn_name"
done

if [[ $missing -gt 0 ]]; then
    echo ""
    echo "FAIL: $missing inner-enum variant(s) not handled at the raw C mapper layer"
    echo "Add an explicit arm to the relevant record_*_error fn in $TST_C_ERROR"
    echo "BEFORE the wildcard _ => arm. If the new variant cannot reach the raw"
    echo "path, add it to the excluded list in scripts/check-raw-c-mapper-coverage.sh"
    echo "with a one-line comment explaining why."
    exit 1
fi

echo "OK: all $total raw inner-enum variant(s) covered across 2 record_*_error fns"
