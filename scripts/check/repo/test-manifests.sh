#!/usr/bin/env bash
# Existence/format-only checker for the advisory coverage manifests under
# tests/coverage/. Enforces that the manifests are well-formed and reference
# real, path-safe, in-tree files. Deliberately NOT a coverage gate — it never
# asserts coverage is "enough" (that is deferred).
#
# Rules:
#   fixture-manifest.toml — each `root` exists, is repo-relative (no leading /,
#       no ..), lives under crates/ or tests/, and carries no_private_corpus=true;
#       each `origin` is one of synthetic|public|derived.
#   skip-ledger.toml      — each `class` is recognized; blocked_bug/placeholder
#       entries carry `expires_after` or `resolved`.
#   stream-matrix.toml    — each axis value is one of covered|partial|gap.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
cd "$ROOT"
COV="tests/coverage"
fail=0
err() { echo "FAIL: $*" >&2; fail=1; }

for f in fixture-manifest.toml skip-ledger.toml stream-matrix.toml README.md; do
    [[ -f "$COV/$f" ]] || err "$COV/$f missing"
done

# --- fixture-manifest.toml --------------------------------------------------
FM="$COV/fixture-manifest.toml"
if [[ -f "$FM" ]]; then
    # roots: exist, repo-relative, under crates/ or tests/
    while IFS= read -r path; do
        case "$path" in
            /*)   err "fixture root is absolute: $path"; continue ;;
            *..*) err "fixture root contains '..': $path"; continue ;;
            crates/*|tests/*) ;;
            *)    err "fixture root outside crates/ or tests/: $path"; continue ;;
        esac
        [[ -e "$path" ]] || err "fixture root does not exist: $path"
    done < <(grep -oE '^root = "[^"]+"' "$FM" | sed -E 's/^root = "(.*)"/\1/')

    # origin values recognized
    while IFS= read -r origin; do
        case "$origin" in
            synthetic|public|derived) ;;
            *) err "unrecognized fixture origin: '$origin'" ;;
        esac
    done < <(grep -oE '^origin = "[^"]+"' "$FM" | sed -E 's/^origin = "(.*)"/\1/')

    # every group asserts no_private_corpus = true
    groups=$(grep -c '^\[\[group\]\]' "$FM" || true)
    flags=$(grep -c '^no_private_corpus = true' "$FM" || true)
    [[ "$groups" -eq "$flags" ]] || err "fixture-manifest: $groups groups but $flags 'no_private_corpus = true' flags"
fi

# --- skip-ledger.toml -------------------------------------------------------
SL="$COV/skip-ledger.toml"
if [[ -f "$SL" ]]; then
    while IFS= read -r class; do
        case "$class" in
            placeholder|external_tool|environmental|slow|diagnostic|blocked_bug) ;;
            *) err "unrecognized skip class: '$class'" ;;
        esac
    done < <(grep -oE '^class = "[^"]+"' "$SL" | sed -E 's/^class = "(.*)"/\1/')

    # blocked_bug / placeholder entries must carry expires_after or resolved.
    need=$(grep -cE '^class = "(blocked_bug|placeholder)"' "$SL" || true)
    have=$(grep -cE '^(expires_after|resolved) =' "$SL" || true)
    [[ "$have" -ge "$need" ]] || err "skip-ledger: $need blocked_bug/placeholder entries but only $have expires_after/resolved markers"
fi

# --- stream-matrix.toml -----------------------------------------------------
SM="$COV/stream-matrix.toml"
if [[ -f "$SM" ]]; then
    # axis values look like `key = "covered"` under [video]/[audio]/etc. Only
    # validate quoted status-like values, ignoring the [gaps] notes array.
    while IFS= read -r val; do
        case "$val" in
            covered|partial|gap) ;;
            *) err "unrecognized stream-matrix status: '$val'" ;;
        esac
    done < <(grep -oE '^[a-z0-9_]+ = "(covered|partial|gap|[a-z]+)"$' "$SM" \
             | sed -E 's/^[a-z0-9_]+ = "(.*)"$/\1/')
fi

if [[ "$fail" -eq 0 ]]; then
    echo "PASS: test-manifests (existence + format)"
fi
exit "$fail"
