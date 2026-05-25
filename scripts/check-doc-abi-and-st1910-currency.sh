#!/usr/bin/env bash
# Plan #96 Wave D ratchet: keep ABI-version docs and ST 1910 citations
# from regressing.
#
# Three rules:
#
#   1. README.md, docs/, and crate-level rustdoc must not mention
#      a stale ABI minor (`ABI version 0.0`..`0.4`). Current value is
#      tracked by TST_ABI_VERSION_MINOR in crates/tst-c/src/lib.rs; the
#      published docs must match.
#
#   2. Bare `ST 1910` (i.e. NOT followed by `.1`) must not appear in
#      crates/ or README.md. The 2026-05-24 audit found 6 sites
#      mis-citing MPEG-TS sync-metadata-AU-cell carriage as "ST 1910";
#      the correct cite is H.222.0 §2.12.4.2 (with ST 1402 as the
#      MISB-side mapping spec). ST 1910.1 itself is a real standard
#      about KLV-in-CMAF-emsg delivery; references with the `.1`
#      version suffix are legitimate (CMAF/HLS deferred-feature
#      context).
#
#   3. tst-c crate-level docs must not say receiver / demux surfaces
#      are "pending" — those surfaces shipped in plan #62 + validate-1.
#
# Bash 3.2-portable: no `mapfile`, no `declare -A`, no `readarray`
# (see feedback_bash_ratchets_macos_portability.md). Uses
# `while IFS= read -r x; do arr+=("$x"); done < <(...)` pattern.

set -euo pipefail

cd "$(dirname "$0")/.."

FAILED=0

# -----------------------------------------------------------------------
# Rule 1 — Stale ABI minor wording
# -----------------------------------------------------------------------
#
# Search README, docs/, and any Rust crate-level rustdoc (//! lines)
# for "ABI version 0.0", "0.1", "0.2", or "0.3".
ABI_HITS=()
while IFS= read -r line; do
    ABI_HITS+=("$line")
done < <(
    grep -rnE 'ABI version 0\.[0-4]([^0-9]|$)' \
        README.md docs/ crates/ 2>/dev/null \
        | { grep -v '^[^:]*\.lock:' || true; } \
        | { grep -v 'check-doc-abi-and-st1910-currency\.sh:' || true; }
)

if [ ${#ABI_HITS[@]} -gt 0 ]; then
    echo "FAIL: stale 'ABI version 0.[0-4]' references found:"
    for h in "${ABI_HITS[@]}"; do echo "  $h"; done
    echo
    echo "Current ABI minor (per crates/tst-c/src/lib.rs TST_ABI_VERSION_MINOR): 5"
    echo "Update each hit to the current value."
    FAILED=1
fi

# -----------------------------------------------------------------------
# Rule 2 — Bare 'ST 1910' (not followed by '.')
# -----------------------------------------------------------------------
#
# Forbid `ST 1910` or `ST1910` NOT followed by a `.` (which would
# indicate a versioned cite like `ST 1910.1`). Hits anywhere in
# crates/ or README.md are presumed mis-cites for MPEG-TS AU cells.
#
# Allowlist: docs/compatibility.md (CMAF section) + docs/deferred-features.md
# (CMAF entry) are scoped out by NOT searching docs/.
ST1910_HITS=()
while IFS= read -r line; do
    # Skip if it's actually ST 1910.<digit>
    if echo "$line" | grep -qE 'ST ?1910\.[0-9]'; then
        continue
    fi
    ST1910_HITS+=("$line")
done < <(
    grep -rnE 'ST ?1910' README.md crates/ 2>/dev/null \
        | { grep -v 'check-doc-abi-and-st1910-currency\.sh:' || true; }
)

if [ ${#ST1910_HITS[@]} -gt 0 ]; then
    echo "FAIL: bare 'ST 1910' (mis-cite for MPEG-TS sync metadata AU cells) found:"
    for h in "${ST1910_HITS[@]}"; do echo "  $h"; done
    echo
    echo "Correct cite for the 5-byte Metadata_AU_cell header is"
    echo "ITU-T H.222.0 §2.12.4.2 (with MISB ST 1402 as the MISB-side"
    echo "mapping spec). ST 1910.1 is a distinct CMAF/HLS standard; use"
    echo "it with the .1 suffix when actually referring to that work."
    FAILED=1
fi

# -----------------------------------------------------------------------
# Rule 3 — tst-c crate docs claiming receiver/demux pending
# -----------------------------------------------------------------------
TSTC_LIB="crates/tst-c/src/lib.rs"
if [ -f "$TSTC_LIB" ]; then
    PENDING_HITS=()
    while IFS= read -r line; do
        PENDING_HITS+=("$line")
    done < <(
        # Crate-level docs are //! lines; only check the first ~30 lines.
        sed -n '1,40p' "$TSTC_LIB" \
            | grep -nE '(receiver[- ]surface|demux event surface).*pending|pending.*(receiver[- ]surface|demux event surface)' \
            || true
    )
    if [ ${#PENDING_HITS[@]} -gt 0 ]; then
        echo "FAIL: $TSTC_LIB crate-level docs still mark receiver/demux surfaces as pending:"
        for h in "${PENDING_HITS[@]}"; do echo "  $TSTC_LIB:$h"; done
        echo
        echo "Receiver-side surfaces (raw, TS-aligned, typed demux event)"
        echo "all shipped — update the crate-level //! docs to current state."
        FAILED=1
    fi
fi

if [ "$FAILED" -ne 0 ]; then
    exit 1
fi

echo "OK: ABI-version docs are current, ST 1910 mis-cites scrubbed, tst-c crate docs reflect shipped state"
