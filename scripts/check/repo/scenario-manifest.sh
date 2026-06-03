#!/usr/bin/env bash
# Ratchet: validate crates/tst-integration scenarios.toml integrity.
#
# Enforced (fails the build on violation):
#   1. Duplicate scenario id.
#   2. A `kind` value not in {demux, roundtrip, binding_contract}.
#   3. A feature token not in {srt, rtp, udp, tcp, hls, rist}.
#   4. An `input` path that does not resolve on disk relative to the manifest dir.
#   5. A `golden` path that does not resolve on disk relative to the manifest dir.
#
# Run it:    bash scripts/check/repo/scenario-manifest.sh
# Self-test: bash scripts/check/repo/scenario-manifest.sh --self-test
#
# Overridable (for the self-test):
#   SCENARIO_MANIFEST   path to the scenarios.toml file
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"

SCENARIO_MANIFEST="${SCENARIO_MANIFEST:-$ROOT/crates/tst-integration/tests/fixtures/scenarios/scenarios.toml}"

# Source-of-truth allow-sets. Extend when a new scenario kind/feature is added to scenarios.toml.
KNOWN_KINDS="demux roundtrip binding_contract"
KNOWN_FEATURES="srt rtp udp tcp hls rist"

# ---------------------------------------------------------------------------
# run_check: validate the manifest pointed to by $SCENARIO_MANIFEST
# ---------------------------------------------------------------------------
run_check() {
  [ -f "$SCENARIO_MANIFEST" ] || { echo "FAIL: manifest not found: $SCENARIO_MANIFEST" >&2; return 1; }
  local DIR fail=0
  DIR="$(dirname "$SCENARIO_MANIFEST")"

  # --- Rule 1: duplicate ids ------------------------------------------------
  # Match both `id = "foo"` (spaced, real manifest) and `id="foo"` (self-test).
  # `|| true`: under `set -euo pipefail`, a no-match grep (exit 1) inside this
  # command substitution would otherwise abort the whole script mid-check with
  # no "FAIL:" message. Treat "no ids found" as an empty set and report it
  # cleanly below (an empty/malformed manifest is exactly what this ratchet
  # should catch, not silently die on).
  local ids
  ids="$(grep -E '^id[[:space:]]*=[[:space:]]*"' "$SCENARIO_MANIFEST" \
         | sed -E 's/^id[[:space:]]*=[[:space:]]*"(.*)"/\1/' || true)"
  if [ -z "$ids" ]; then
    echo "FAIL: manifest contains no scenario ids: $SCENARIO_MANIFEST" >&2
    fail=1
  fi
  local dups
  dups="$(printf '%s\n' "$ids" | sort | uniq -d)"
  if [ -n "$dups" ]; then
    echo "FAIL: duplicate scenario id(s): $(printf '%s\n' "$dups" | tr '\n' ' ')" >&2
    fail=1
  fi

  # --- Rule 2: unknown kind -------------------------------------------------
  local kind
  while IFS= read -r kind; do
    [ -n "$kind" ] || continue
    if ! printf '%s\n' $KNOWN_KINDS | grep -Fxq "$kind"; then
      echo "FAIL: unknown kind: $kind" >&2
      fail=1
    fi
  done < <(grep -E '^kind[[:space:]]*=[[:space:]]*"' "$SCENARIO_MANIFEST" \
            | sed -E 's/^kind[[:space:]]*=[[:space:]]*"(.*)"/\1/')

  # --- Rule 3: unknown feature tokens ---------------------------------------
  # features line may be `features = []` or `features = ["srt", "rtp"]`
  local feat_line tok tokens
  while IFS= read -r feat_line; do
    # Strip the key prefix, brackets, quotes, and spaces; split on commas.
    feat_line="$(printf '%s' "$feat_line" \
                 | sed -E 's/^features[[:space:]]*=[[:space:]]*//' \
                 | tr -d '[]" ')"
    # Empty array is fine.
    [ -n "$feat_line" ] || continue
    # Split comma-separated tokens (IFS scoped to this read, no save/restore).
    IFS=',' read -ra tokens <<< "$feat_line"
    for tok in "${tokens[@]}"; do
      [ -n "$tok" ] || continue
      if ! printf '%s\n' $KNOWN_FEATURES | grep -Fxq "$tok"; then
        echo "FAIL: unknown feature token: $tok" >&2
        fail=1
      fi
    done
  done < <(grep -E '^features[[:space:]]*=[[:space:]]*\[' "$SCENARIO_MANIFEST")

  # --- Rule 4: input paths must exist ---------------------------------------
  local inp
  while IFS= read -r inp; do
    [ -n "$inp" ] || continue
    if [ ! -f "$DIR/$inp" ]; then
      echo "FAIL: missing input file: $inp" >&2
      fail=1
    fi
  done < <(grep -E '^input[[:space:]]*=[[:space:]]*"' "$SCENARIO_MANIFEST" \
            | sed -E 's/^input[[:space:]]*=[[:space:]]*"(.*)"/\1/')

  # --- Rule 5: golden paths must exist --------------------------------------
  local gld
  while IFS= read -r gld; do
    [ -n "$gld" ] || continue
    if [ ! -f "$DIR/$gld" ]; then
      echo "FAIL: missing golden file: $gld" >&2
      fail=1
    fi
  done < <(grep -E '^golden[[:space:]]*=[[:space:]]*"' "$SCENARIO_MANIFEST" \
            | sed -E 's/^golden[[:space:]]*=[[:space:]]*"(.*)"/\1/')

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
    if run_check >/dev/null 2>&1; then rc=pass; else rc=fail; fi
    if [ "$rc" = "$want" ]; then
      echo "  ok: $label (expected $want)"
    else
      echo "  SELF-TEST FAIL: $label expected $want got $rc" >&2
      return 1
    fi
  }

  export SCENARIO_MANIFEST="$tmp/scenarios.toml"

  # Lay down real fixture stubs that the valid manifest will reference.
  mkdir -p "$tmp/s"
  : > "$tmp/s/in.ts"
  printf '{}' > "$tmp/s/g.json"

  # (1) Valid minimal manifest — must pass.
  cat > "$SCENARIO_MANIFEST" <<'TOML'
[[scenario]]
id = "a"
kind = "demux"
input = "s/in.ts"
golden = "s/g.json"
features = []
tier = "A"
schema_version = 0
TOML
  expect pass "valid single-scenario manifest" || return 1

  # (2) Duplicate id — must fail.
  cat > "$SCENARIO_MANIFEST" <<'TOML'
[[scenario]]
id = "a"
kind = "demux"
input = "s/in.ts"
golden = "s/g.json"
features = []
tier = "A"
schema_version = 0

[[scenario]]
id = "a"
kind = "roundtrip"
input = "s/in.ts"
golden = "s/g.json"
features = []
tier = "A"
schema_version = 0
TOML
  expect fail "duplicate id" || return 1

  # (3) Unknown kind — must fail.
  cat > "$SCENARIO_MANIFEST" <<'TOML'
[[scenario]]
id = "b"
kind = "badkind"
input = "s/in.ts"
golden = "s/g.json"
features = []
tier = "A"
schema_version = 0
TOML
  expect fail "unknown kind" || return 1

  # (4) Unknown feature token — must fail.
  cat > "$SCENARIO_MANIFEST" <<'TOML'
[[scenario]]
id = "c"
kind = "demux"
input = "s/in.ts"
golden = "s/g.json"
features = ["srt", "quic"]
tier = "A"
schema_version = 0
TOML
  expect fail "unknown feature token" || return 1

  # (5) Missing input file — must fail.
  cat > "$SCENARIO_MANIFEST" <<'TOML'
[[scenario]]
id = "d"
kind = "demux"
input = "s/no-such-input.ts"
golden = "s/g.json"
features = []
tier = "A"
schema_version = 0
TOML
  expect fail "missing input file" || return 1

  # (6) Missing golden file — must fail.
  cat > "$SCENARIO_MANIFEST" <<'TOML'
[[scenario]]
id = "e"
kind = "demux"
input = "s/in.ts"
golden = "s/no-such-golden.json"
features = []
tier = "A"
schema_version = 0
TOML
  expect fail "missing golden file" || return 1

  # (7) Empty / no-id manifest — must fail CLEANLY (not abort the script under
  # `set -euo pipefail` when the id grep finds no matches).
  printf '# manifest with no scenarios\n' > "$SCENARIO_MANIFEST"
  expect fail "manifest with no scenario ids" || return 1

  echo "self-test: PASS"
}

# ---------------------------------------------------------------------------
# main
# ---------------------------------------------------------------------------
if [ "${1:-}" = "--self-test" ]; then
  self_test
else
  # Prove the checker still catches its negatives before trusting a pass.
  # Subshell so the self-test's SCENARIO_MANIFEST override can't leak into run_check.
  ( self_test ) >/dev/null
  run_check
  echo "scenario-manifest: OK"
fi
