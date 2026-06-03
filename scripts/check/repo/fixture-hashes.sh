#!/usr/bin/env bash
# Ratchet: enforce full-closure SHA-256 provenance for committed fixtures.
#   - every git-tracked file under each fixture-manifest.toml group root must
#     appear exactly once in fixture-hashes.toml;
#   - every listed file must exist, be repo-relative (no leading /, no ..), and
#     its recomputed SHA-256 must match;
#   - no listed file may be untracked/removed.
# Regenerate with scripts/gen/fixture-manifest.sh after intentional changes.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
cd "$ROOT"
FM="tests/coverage/fixture-manifest.toml"
HASHES="tests/coverage/fixture-hashes.toml"
fail=0
err() { echo "FAIL: $*" >&2; fail=1; }

[[ -f "$HASHES" ]] || { echo "FAIL: $HASHES missing — run scripts/gen/fixture-manifest.sh" >&2; exit 1; }

# Both temps created before arming the trap (so an early exit under `set -u`
# can't hit an unbound $listed and leak $expected).
expected="$(mktemp)"
listed="$(mktemp)"
trap 'rm -f "$expected" "$listed"' EXIT

# Expected set: every tracked file under every group root.
grep -oE '^root = "[^"]+"' "$FM" | sed -E 's/^root = "(.*)"/\1/' | while IFS= read -r root; do
    git ls-files -- "$root"
done | LC_ALL=C sort -u > "$expected" || true
[[ -s "$expected" ]] || err "no fixture roots resolved from $FM"

# Listed set + per-file hash verification.
path=""
while IFS= read -r line; do
    case "$line" in
        'path = '*)
            path="$(printf '%s' "$line" | sed -E 's/^path = "(.*)"/\1/')"
            case "$path" in
                /*)   err "listed path is absolute: $path"; path=""; continue ;;
                *..*) err "listed path contains '..': $path"; path=""; continue ;;
            esac
            echo "$path" >> "$listed"
            [[ -f "$path" ]] || err "listed file does not exist: $path" ;;
        'sha256 = '*)
            [[ -n "$path" && -f "$path" ]] || { path=""; continue; }
            want="$(printf '%s' "$line" | sed -E 's/^sha256 = "(.*)"/\1/')"
            got="$(sha256sum "$path" | cut -d' ' -f1)"
            [[ "$got" == "$want" ]] || err "sha256 drift for $path (have $got, manifest $want)"
            path="" ;;
    esac
done < "$HASHES"
LC_ALL=C sort -u -o "$listed" "$listed"

# Closure both ways.
while IFS= read -r p; do grep -qxF "$p" "$listed" || err "unlisted committed fixture: $p (run scripts/gen/fixture-manifest.sh)"; done < "$expected"
while IFS= read -r p; do grep -qxF "$p" "$expected" || err "listed file no longer tracked under a fixture root: $p"; done < "$listed"

[[ "$fail" -eq 0 ]] && echo "PASS: fixture-hashes (closure + sha256)"
exit "$fail"
