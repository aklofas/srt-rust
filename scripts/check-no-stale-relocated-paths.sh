#!/usr/bin/env bash
# Fail if any reference to a RELOCATED-AWAY package directory survives in the
# tracked tree. These directories were moved during the 2026-06-02 refactor
# series and no longer exist, so a literal reference to them is a silent dead
# link, a stale doc path, or (worse) a broken build trigger:
#
#   crates/tst-c, crates/tst-c-core, crates/tst-py   -> bindings/ (relocation)
#   bindings/c/tst-c, bindings/c/tst-c-core          -> bindings/c{,/core} (Option-B flatten)
#   crates/baremetal-qemu, crates/baremetal-qemu-c   -> embedded/ (embedded move)
#
# This rail exists because the moves above are path-coupled and the literal
# grep used during each move is BLIND to a few forms (slashless `cd crates/tst-c`,
# relative `../tst-c-core` build triggers). This catches the literal-path class
# at CI time so it can't silently rot. See memory project_bindings_relocation_shipped
# + feedback_crate_move_relative_path_walks.
#
# CHANGELOG.md is exempt: it is an append-only history and its entries were
# accurate at the time they were written.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

# Removed path prefixes. The leaf forms `crates/tst-c` and `bindings/c/tst-c`
# carry a trailing-boundary class so they do NOT match the still-present
# `crates/tst-core` (next char `o`) or `bindings/c/core` (no `tst-c`). The
# `*-core` removed dirs are matched explicitly (longest-first), and `baremetal-qemu`
# covers both `baremetal-qemu` and `baremetal-qemu-c`.
PATTERN='crates/tst-c-core|crates/tst-py|crates/baremetal-qemu|bindings/c/tst-c-core|(crates|bindings/c)/tst-c([/")`, ]|$)'

# Exempt: CHANGELOG (history) and this script (it names the forbidden paths).
hits=$(git ls-files \
  | grep -vE '^(CHANGELOG\.md|scripts/check-no-stale-relocated-paths\.sh)$' \
  | tr '\n' '\0' \
  | xargs -0 grep -InE "$PATTERN" 2>/dev/null || true)

if [ -n "$hits" ]; then
  echo "FAIL: references to relocated-away package directories found (these dirs no longer exist):" >&2
  echo "$hits" >&2
  echo "" >&2
  echo "Update each to its current location:" >&2
  echo "  crates/tst-c, crates/tst-c-core, crates/tst-py  -> bindings/c, bindings/c/core, bindings/python" >&2
  echo "  bindings/c/tst-c, bindings/c/tst-c-core         -> bindings/c, bindings/c/core" >&2
  echo "  crates/baremetal-qemu{,-c}                      -> embedded/baremetal-qemu{,-c}" >&2
  echo "(CHANGELOG.md is exempt as historical record.)" >&2
  exit 1
fi

echo "OK: no stale references to relocated-away package directories"
