#!/usr/bin/env bash
# 30th bash ratchet (Plan A5a Wave C).
# Verifies every method on the tst_core::publisher::Publisher trait has a
# corresponding tst_publisher_* C entry point in bindings/c/tst-c-core/src/hls/ AND
# appears in the generated bindings/c/tst-c/include/tstrans.h.
#
# Catches future drift: when someone adds a new method to the Publisher trait,
# this ratchet fails until they ALSO add a matching C entry point in src/hls/
# AND update the EXPECTED_SYMS map below.

set -euo pipefail

CORE_TRAIT="crates/tst-core/src/publisher/mod.rs"
HLS_DIR="bindings/c/tst-c-core/src/hls"
HEADER="bindings/c/tst-c/include/tstrans.h"

if [[ ! -f "$CORE_TRAIT" ]]; then
    echo "FAIL: $CORE_TRAIT not found"
    exit 1
fi
if [[ ! -d "$HLS_DIR" ]]; then
    echo "FAIL: $HLS_DIR not found"
    exit 1
fi
if [[ ! -f "$HEADER" ]]; then
    echo "FAIL: $HEADER not found"
    exit 1
fi

# Extract Publisher trait method names.
# The trait body contains lines like `    fn push_ts(...` and `    fn finish(self`.
# We extract only methods declared inside `pub trait Publisher { ... }`.
TRAIT_METHODS=$(awk '/pub trait Publisher/,/^}/' "$CORE_TRAIT" | \
    grep -oE '^\s+fn ([a-z_]+)[(<]' | \
    sed -E 's/[[:space:]]*fn ([a-z_]+).*/\1/' | sort -u)

if [[ -z "$TRAIT_METHODS" ]]; then
    echo "FAIL: could not extract any methods from Publisher trait in $CORE_TRAIT"
    exit 1
fi

# Hardcoded map: trait method name -> required C symbol.
# bash 3.2 (the /bin/bash macOS still ships) has no associative arrays, so a
# `case` lookup stands in for `declare -A`.
# MAINTAINER NOTE: When a new method is added to the Publisher trait, this
# ratchet will fail. To fix: add a new C entry point in src/hls/ (and ensure
# cbindgen regenerates the header), then extend this map.
expected_sym() { # <trait method> -> required C symbol, or empty if undocumented
    case "$1" in
        push_ts)     echo "tst_publisher_push_ts" ;;
        cut_segment) echo "tst_publisher_cut_segment" ;;
        finish)      echo "tst_publisher_finish" ;;
        stats)       echo "tst_publisher_get_stats" ;;
        *)           echo "" ;;
    esac
}

MISSING=""
for m in $TRAIT_METHODS; do
    sym="$(expected_sym "$m")"
    if [[ -z "$sym" ]]; then
        MISSING="${MISSING}  - trait method '${m}' has no documented C mirror in expected_sym"$'\n'
        MISSING="${MISSING}    (add a case to expected_sym in this script AND add the C entry point)"$'\n'
        continue
    fi
    if ! grep -qr "$sym" "$HLS_DIR"/; then
        MISSING="${MISSING}  - trait method '${m}': C entry point '${sym}' not found in ${HLS_DIR}/"$'\n'
    fi
    if ! grep -q "$sym" "$HEADER"; then
        MISSING="${MISSING}  - trait method '${m}': symbol '${sym}' missing from generated header ${HEADER}"$'\n'
    fi
done

if [[ -n "$MISSING" ]]; then
    printf "FAIL: publisher-trait-mirror:\n%s" "$MISSING"
    exit 1
fi

echo "PASS: publisher-trait-mirror"
