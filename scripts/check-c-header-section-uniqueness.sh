#!/usr/bin/env bash
# Verify that bindings/c/tst-c/include/tstrans.h has between 7 and 9 section
# dividers (// ─── NAME ──────) and that every divider name is unique.
#
# Plan B's add_section_dividers post-process is specified to emit 7
# required sections (INTROSPECTION, MUX SENDER, TS SENDER, RAW SENDER,
# DEMUX RECEIVER, TS RECEIVER, RAW RECEIVER) plus 2 conditional
# catch-alls (LIFETIME, OTHER) that only emit when non-empty.
#
# A regression here means add_section_dividers reverted to the
# line-by-line transition-emission shape from before the Codex Wave 6
# review fix (docs/refactor-1/_codex-wave-6-implementation-validation.md
# Finding 1, 2026-05-19), which produces 16 dividers with 7 sections
# duplicated against cbindgen's name-sorted output.

set -euo pipefail

cd "$(dirname "$0")/.."

HEADER="bindings/c/tst-c/include/tstrans.h"
if [ ! -f "$HEADER" ]; then
    echo "FAIL: header not found at $HEADER"
    exit 1
fi

DIVIDERS=$(grep -E '^// ─── ' "$HEADER" || true)
COUNT=$(printf '%s\n' "$DIVIDERS" | grep -c . || true)

if [ "$COUNT" -lt 7 ] || [ "$COUNT" -gt 9 ]; then
    echo "FAIL: $HEADER has $COUNT section dividers; expected 7-9."
    echo "Plan B specifies 7 required + up to 2 conditional sections."
    echo "Observed dividers:"
    printf '%s\n' "$DIVIDERS" | sed 's/^/  /'
    exit 1
fi

NAMES=$(printf '%s\n' "$DIVIDERS" | sed 's/^\/\/ ─── //' | sed 's/ ─.*$//')
UNIQUE=$(printf '%s\n' "$NAMES" | sort -u | grep -c . || true)

if [ "$UNIQUE" -ne "$COUNT" ]; then
    DUPES=$(printf '%s\n' "$NAMES" | sort | uniq -d)
    echo "FAIL: $HEADER has duplicate section divider names:"
    printf '%s\n' "$DUPES" | sed 's/^/  /'
    echo
    echo "add_section_dividers in bindings/c/tst-c/build.rs should emit each"
    echo "section at most once. A duplicate name signals the post-process"
    echo "regressed to line-by-line transition emission against sort_by=Name."
    exit 1
fi

echo "OK: $HEADER has $COUNT unique section dividers"
