#!/usr/bin/env bash
# L3 surface-manifest ratchet.
#
# Enforced (fails the build on violation):
#   (a) every owning_tests path in tests/coverage/surface-manifest.toml exists on disk;
#   (b) every binding column symbol resolves in that binding's source
#       (feature-tagged "[feature=X]" entries are skipped unless X is in BUILT_FEATURES);
#   (c) closure: every mappable public-api.txt item is mapped ([[surface]] item)
#       or exempted ([[exempt]] item).
#
# Run it:    bash scripts/check/repo/surface-manifest.sh
# Self-test: bash scripts/check/repo/surface-manifest.sh --self-test
#
# Overridable (for the self-test):
#   SURFACE_MANIFEST        path to the manifest TOML
#   SURFACE_BASELINE_DIR    directory containing <crate>/public-api.txt files
#   SURFACE_CRATES          space-separated list of crate names to check
#   SURFACE_BUILT_FEATURES  space-separated features considered built (default: all)
#   SURFACE_C_HEADER        path to the C binding header
#   SURFACE_PYI_DIR         directory to search for Python binding symbols
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"

SURFACE_MANIFEST="${SURFACE_MANIFEST:-$ROOT/tests/coverage/surface-manifest.toml}"
SURFACE_BASELINE_DIR="${SURFACE_BASELINE_DIR:-$ROOT/crates}"
SURFACE_CRATES="${SURFACE_CRATES:-rist-sys tst-core tst-pipeline tst-rist tst-rtp tst-srt tst-tcp tst-udp}"
SURFACE_BUILT_FEATURES="${SURFACE_BUILT_FEATURES:-srt rtp udp tcp hls rist}"
SURFACE_C_HEADER="${SURFACE_C_HEADER:-$ROOT/bindings/c/include/tstrans.h}"
SURFACE_PYI_DIR="${SURFACE_PYI_DIR:-$ROOT/bindings/python/python/tstrans}"

# ---------------------------------------------------------------------------
# extract_items: stdin is a public-api.txt; stdout is one canonical key per
# mappable line. Mappable = pub fn/struct/enum/trait/type/const (incl. const fn).
# auto-derived impl lines are NOT mappable and are skipped.
# ---------------------------------------------------------------------------
extract_items() {
  awk '
    /^pub (const fn|fn|struct|enum|trait|type|const) / {
      line=$0
      sub(/^pub const fn /, "pub fn ", line)           # normalize "pub const fn"
      sub(/^pub (fn|struct|enum|trait|type|const) /, "", line)
      sub(/[(<].*$/, "", line)                         # drop fn args / generics
      sub(/[[:space:]]*=.*$/, "", line)                # drop type alias RHS: " = ..."
      sub(/:[[:space:]].*$/, "", line)                 # drop ": Type" annotation
      sub(/[[:space:]]+$/, "", line)
      if (line != "") print line
    }
  '
}

# ---------------------------------------------------------------------------
# run_check: validate the manifest against the baselines and binding sources
# ---------------------------------------------------------------------------
run_check() {
  [ -f "$SURFACE_MANIFEST" ] || { echo "FAIL: manifest not found: $SURFACE_MANIFEST" >&2; return 1; }
  local errs; errs="$(mktemp)"; trap 'rm -f "$errs"' RETURN
  local fail=0

  local mapped exempt
  mapped="$(mktemp)"; exempt="$(mktemp)"
  # All "item = " lines in [[surface]] AND [[exempt]] sections
  grep -E '^item = ' "$SURFACE_MANIFEST" | sed -E 's/^item = "(.*)"/\1/' | sort -u > "$mapped"
  # Only items from [[exempt]] sections (the awk skips [[surface]] items)
  awk '
    /^\[\[exempt\]\]/ { e=1; next }
    /^\[\[surface\]\]/ { e=0; next }
    e && /^item = / { sub(/^item = "/, ""); sub(/".*$/, ""); print }
  ' "$SURFACE_MANIFEST" | sort -u > "$exempt"

  # (a) owning_tests paths exist.
  local p
  grep -E '^owning_tests = ' "$SURFACE_MANIFEST" | grep -oE '"[^"]+"' | tr -d '"' \
  | while IFS= read -r p; do
    [ -f "$ROOT/$p" ] || echo "FAIL: owning test path missing: $p"
  done >> "$errs"

  # (b) binding symbols resolve (feature-aware).
  local b feat sym leaf
  grep -E '^bindings = ' "$SURFACE_MANIFEST" | grep -oE '"[^"]+"' | tr -d '"' \
  | while IFS= read -r b; do
    feat=""
    sym="$b"
    # Extract optional [feature=X] suffix
    if printf '%s' "$b" | grep -q '\[feature='; then
      feat="$(printf '%s' "$b" | sed -E 's/.*\[feature=([a-z]+)\].*/\1/')"
      sym="$(printf '%s' "$b" | sed -E 's/ *\[feature=[a-z]+\]//')"
      # Skip if feature not built
      printf '%s\n' $SURFACE_BUILT_FEATURES | grep -Fxq "$feat" || continue
    fi
    case "$sym" in
      c:*)
        grep -Fq "${sym#c:}" "$SURFACE_C_HEADER" \
          || echo "FAIL: c symbol unresolved: ${sym#c:}" ;;
      python:*)
        leaf="${sym#python:}"
        leaf="${leaf##*.}"   # last dotted component
        grep -rFq "$leaf" "$SURFACE_PYI_DIR" \
          || echo "FAIL: python symbol unresolved: ${sym#python:} (leaf: $leaf)" ;;
      java:*|swift:*|kotlin:*)
        : ;;   # RESERVED until tst-jni / tst-uniffi land; presence-only, not resolved
      *)
        echo "FAIL: unknown binding prefix: $sym" ;;
    esac
  done >> "$errs"

  # (c) closure: every mappable baseline item is mapped or exempted.
  local c f item
  for c in $SURFACE_CRATES; do
    f="$SURFACE_BASELINE_DIR/$c/public-api.txt"; [ -f "$f" ] || continue
    while IFS= read -r item; do
      [ -n "$item" ] || continue
      grep -Fxq "$item" "$mapped" && continue
      grep -Fxq "$item" "$exempt" && continue
      echo "FAIL: un-catalogued public item (add to [[surface]] or [[exempt]] in surface-manifest.toml): $item"
    done < <(extract_items < "$f")
  done >> "$errs"

  rm -f "$mapped" "$exempt"

  if [ -s "$errs" ]; then
    cat "$errs" >&2
    fail=1
  fi
  return "$fail"
}

# ---------------------------------------------------------------------------
# self_test: build throwaway manifests and assert each negative is caught
# ---------------------------------------------------------------------------
self_test() {
  local tmp; tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' RETURN
  local rc

  expect() { # <expected pass|fail> <label>
    local want="$1" label="$2"
    if ( cd "$ROOT" && run_check ) >/dev/null 2>&1; then rc=pass; else rc=fail; fi
    if [ "$rc" = "$want" ]; then
      echo "  ok: $label (expected $want)"
    else
      echo "  SELF-TEST FAIL: $label expected $want got $rc" >&2
      return 1
    fi
  }

  # Set up an isolated fake tree
  mkdir -p "$tmp/crates/demo"
  printf 'pub fn demo::a()\npub struct demo::B\n' > "$tmp/crates/demo/public-api.txt"
  : > "$tmp/h.h"   # empty C header

  export SURFACE_BASELINE_DIR="$tmp/crates"
  export SURFACE_CRATES="demo"
  export SURFACE_MANIFEST="$tmp/m.toml"
  export SURFACE_C_HEADER="$tmp/h.h"
  export SURFACE_PYI_DIR="$tmp"
  export SURFACE_BUILT_FEATURES="srt"

  # (1) Both items exempted -> pass
  printf '[[exempt]]\nitem = "demo::a"\n[[exempt]]\nitem = "demo::B"\n' > "$tmp/m.toml"
  expect pass "all exempted" || return 1

  # (2) One item neither mapped nor exempted -> fail (closure)
  printf '[[exempt]]\nitem = "demo::a"\n' > "$tmp/m.toml"
  expect fail "un-catalogued item" || return 1

  # (3) Mapped row with a missing owning test -> fail
  printf '[[surface]]\nitem = "demo::a"\nowning_tests = ["tests/coverage/NO_SUCH_FILE.rs"]\nbindings = []\n[[exempt]]\nitem = "demo::B"\n' > "$tmp/m.toml"
  expect fail "missing owning test" || return 1

  # (4) Mapped row with owning test present but unresolved c symbol -> fail
  # Use a real repo-relative path that exists (tests/coverage/README.md)
  printf '[[surface]]\nitem = "demo::a"\nowning_tests = ["tests/coverage/README.md"]\nbindings = ["c:nope_sym_xyz"]\n[[exempt]]\nitem = "demo::B"\n' > "$tmp/m.toml"
  expect fail "unresolved c symbol" || return 1

  echo "self-test: PASS"
}

# ---------------------------------------------------------------------------
# main
# ---------------------------------------------------------------------------
if [ "${1:-}" = "--self-test" ]; then
  self_test
else
  # Prove the checker still catches its negatives before trusting a pass.
  # Subshell so the self-test's env overrides can't leak into run_check.
  ( self_test ) >/dev/null
  run_check
  echo "surface-manifest: OK"
fi
