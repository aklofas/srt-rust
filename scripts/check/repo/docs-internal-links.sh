#!/usr/bin/env bash
# Docs deep-edit rail (2026-07-12): every intra-repo markdown link in
# README.md, docs/**/*.md, and examples/**/*.md must resolve to an existing
# file or directory. Guards the cookbook re-slug and all future doc moves.
#
# Handled link forms:
#   [text](/docs/foo.md)       leading slash    = repo-root-relative
#   [text](../guides/foo.md)   relative         = resolved against the file's dir
#   [text](foo.md#anchor)      fragment         = stripped before the check
#   [text](#anchor)            pure fragment    = skipped
#   http(s)://…, mailto:…      external         = skipped
#
# CHANGELOG.md is deliberately NOT scanned: historical entries legitimately
# reference paths as they existed at the time.
#
# Bash 3.2-portable: no mapfile, no declare -A (macOS ships bash 3.2).

set -euo pipefail
cd "$(dirname "$0")/../../.."

FAILED=0
CHECKED=0

while IFS= read -r src; do
    dir=$(dirname "$src")
    while IFS= read -r target; do
        case "$target" in
            http://*|https://*|mailto:*|'#'*) continue ;;
        esac
        target="${target%%#*}"     # strip fragment
        target="${target%% *}"     # strip '"title"' suffix
        [ -z "$target" ] && continue
        if [ "${target#/}" != "$target" ]; then
            resolved=".${target}"
        else
            resolved="${dir}/${target}"
        fi
        CHECKED=$((CHECKED + 1))
        if [ ! -e "$resolved" ]; then
            echo "FAIL: ${src}: broken link -> ${target}"
            FAILED=1
        fi
    done < <(grep -oE '\]\([^)]+\)' "$src" 2>/dev/null | sed -e 's/^](//' -e 's/)$//')
done < <(find README.md docs examples -name '*.md' -type f 2>/dev/null)

if [ "$FAILED" -ne 0 ]; then
    exit 1
fi
echo "OK: ${CHECKED} intra-repo markdown links resolve"
