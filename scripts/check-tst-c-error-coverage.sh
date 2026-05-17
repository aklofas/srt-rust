#!/usr/bin/env bash
# Verify every variant of upstream pipeline error enums is explicitly
# mapped in the relevant `record_*_error` production function body in
# crates/tst-c/src/error.rs, BEFORE the function's wildcard arm.
#
# Upstream enums are marked #[non_exhaustive], requiring tst-c match
# expressions to include a wildcard arm. Rust's exhaustiveness checker
# cannot tell us when a new upstream variant has been added because the
# wildcard always matches. This ratchet fills that gap: parse upstream
# variant names, parse each production function body up to its wildcard
# arm, and require every upstream variant to appear in the matched
# slice. Fail loudly otherwise.
#
# Style matches scripts/check-no-public-usize.sh +
# check-close-contract-presence.sh: set -euo pipefail; plain FAIL: / OK:
# output; exit 1 on missing, 0 on success.

set -euo pipefail

TST_C_ERROR="crates/tst-c/src/error.rs"

# Each row: enum-full-path | source-file | enum-short-name | record-fn-name
# The record function is the production mapping site to scope the variant
# check to.
ENUMS=(
    "tst_core::error::MuxError|crates/tst-core/src/error.rs|MuxError|record_mux_error"
    "tst_core::transport::TransportError|crates/tst-core/src/transport.rs|TransportError|record_transport_error"
    "tst_pipeline::MuxSenderError|crates/tst-pipeline/src/mux_sender.rs|MuxSenderError|record_sender_error"
    "tst_pipeline::sender::SenderError|crates/tst-pipeline/src/sender/mod.rs|SenderError|record_ts_sender_error"
)

# Extract the body of a function up to (and excluding) its first wildcard
# `_ =>` match arm. The wildcard signals the end of explicit variant
# coverage. Anything beyond `_ =>` does NOT count toward variant coverage.
#
# Implementation: find the line containing `fn <name>(`, then track brace
# depth to find the matching closing brace. Within that range, stop at
# the first line whose trimmed content starts with `_ =>`.
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

# Extract variant names from `pub enum <name> { ... }` block. Captures
# variant identifiers regardless of unit / tuple / struct shape.
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
total_variants=0

for spec in "${ENUMS[@]}"; do
    full_name="${spec%%|*}"
    rest="${spec#*|}"
    enum_file="${rest%%|*}"
    rest="${rest#*|}"
    enum_short="${rest%%|*}"
    fn_name="${rest##*|}"

    variants=$(extract_enum_variants "$enum_file" "$enum_short")

    if [[ -z "$variants" ]]; then
        echo "WARN: $full_name in $enum_file produced zero variants — check enum location"
        continue
    fi

    fn_body=$(extract_function_body_before_wildcard "$TST_C_ERROR" "$fn_name")

    if [[ -z "$fn_body" ]]; then
        echo "FAIL: could not locate fn $fn_name in $TST_C_ERROR"
        exit 1
    fi

    variant_count=0
    for v in $variants; do
        variant_count=$((variant_count + 1))
        total_variants=$((total_variants + 1))
        # Require `EnumShort::Variant` to appear in fn body before wildcard.
        if ! echo "$fn_body" | grep -q "$enum_short::$v\b"; then
            echo "MISSING: $enum_short::$v not handled in $fn_name (before wildcard) in $TST_C_ERROR"
            missing=$((missing + 1))
        fi
    done

    echo "checked $enum_short ($variant_count variants) → $fn_name"
done

if [[ $missing -gt 0 ]]; then
    echo ""
    echo "FAIL: $missing upstream error variant(s) not handled at the C ABI"
    echo "Extend the relevant match in $TST_C_ERROR (record_mux_error /"
    echo "record_transport_error / record_sender_error / record_ts_sender_error)"
    echo "to handle the variant explicitly BEFORE the wildcard \`_ =>\` arm."
    exit 1
fi

echo "OK: all $total_variants upstream error enum variant(s) handled across 4 record_*_error fns"
