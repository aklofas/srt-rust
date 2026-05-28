#!/usr/bin/env bash
# 30th bash ratchet (Plan A5a Wave C).
# Verifies every method on the tst_core::publisher::Publisher trait has a
# corresponding tst_publisher_* C entry point in crates/tst-c/src/hls/ AND
# appears in the generated crates/tst-c/include/tstrans.h.
#
# Catches future drift: when someone adds a new method to the Publisher trait,
# this ratchet fails until they ALSO add a matching C entry point in src/hls/
# AND update the EXPECTED_SYMS map below.

set -euo pipefail

CORE_TRAIT="crates/tst-core/src/publisher/mod.rs"
HLS_DIR="crates/tst-c/src/hls"
HEADER="crates/tst-c/include/tstrans.h"

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
# MAINTAINER NOTE: When a new method is added to the Publisher trait, this
# ratchet will fail. To fix: add a new C entry point in src/hls/ (and ensure
# cbindgen regenerates the header), then extend this map.
declare -A EXPECTED_SYMS
EXPECTED_SYMS[push_ts]="tst_publisher_push_ts"
EXPECTED_SYMS[cut_segment]="tst_publisher_cut_segment"
EXPECTED_SYMS[finish]="tst_publisher_finish"
EXPECTED_SYMS[stats]="tst_publisher_get_stats"

MISSING=""
for m in $TRAIT_METHODS; do
    if [[ -z "${EXPECTED_SYMS[$m]:-}" ]]; then
        MISSING="${MISSING}  - trait method '${m}' has no documented C mirror in EXPECTED_SYMS"$'\n'
        MISSING="${MISSING}    (add an entry to EXPECTED_SYMS in this script AND add the C entry point)"$'\n'
        continue
    fi
    sym="${EXPECTED_SYMS[$m]}"
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
