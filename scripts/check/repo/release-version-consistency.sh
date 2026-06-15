#!/usr/bin/env bash
# REL-01 release-version-consistency ratchet.
#
# Asserts the project version is identical across every source-of-truth so a
# git tag (e.g. v0.2.0) can never publish a Maven/PyPI artifact whose source
# version silently disagrees with the tag.
#
# Six sources are checked (the ONLY version sites in the tree):
#   1. workspace Cargo        Cargo.toml                      [workspace.package] version
#   2. pyproject              bindings/python/pyproject.toml  [project] version
#   3. tst-py crate           bindings/python/Cargo.toml      [package] version
#   4. C version constants    bindings/c/core/src/lib.rs      TST_VERSION_{MAJOR,MINOR,PATCH}
#   5. C committed header     bindings/c/include/tstrans.h    TST_VERSION_{MAJOR,MINOR,PATCH}
#   6. Python version test    bindings/python/tests/test_version.py  tstrans.__version__
#
# NOTE: bindings/jvm/Cargo.toml and bindings/c/core/Cargo.toml use
# `version.workspace = true`, so they equal the workspace version BY
# CONSTRUCTION — they are intentionally not re-checked here.
#
# CRITICAL: sources 4 & 5 anchor on `TST_VERSION_` so they do NOT match the
# unrelated C ABI version (`TST_ABI_VERSION_*`, currently 0.13) — a different
# number that must be ignored.
#
# Run it:    bash scripts/check/repo/release-version-consistency.sh
# Tag check: bash scripts/check/repo/release-version-consistency.sh 0.2.0
#            (or, on a v* tag build, GITHUB_REF_NAME=vX.Y.Z is honoured)
# Self-test: bash scripts/check/repo/release-version-consistency.sh --self-test
#
# Overridable (for the self-test, which points these at temp fixtures):
#   RVC_CARGO_TOML   workspace Cargo.toml
#   RVC_PYPROJECT    bindings/python/pyproject.toml
#   RVC_PY_CARGO     bindings/python/Cargo.toml
#   RVC_C_LIB        bindings/c/core/src/lib.rs
#   RVC_C_HEADER     bindings/c/include/tstrans.h
#   RVC_PY_TEST      bindings/python/tests/test_version.py
#
# Bash 3.2-portable: no `mapfile`/`readarray`/`declare -A`/`declare -a`
# (see feedback_bash_ratchets_macos_portability.md). Does not read stdin.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"

RVC_CARGO_TOML="${RVC_CARGO_TOML:-$ROOT/Cargo.toml}"
RVC_PYPROJECT="${RVC_PYPROJECT:-$ROOT/bindings/python/pyproject.toml}"
RVC_PY_CARGO="${RVC_PY_CARGO:-$ROOT/bindings/python/Cargo.toml}"
RVC_C_LIB="${RVC_C_LIB:-$ROOT/bindings/c/core/src/lib.rs}"
RVC_C_HEADER="${RVC_C_HEADER:-$ROOT/bindings/c/include/tstrans.h}"
RVC_PY_TEST="${RVC_PY_TEST:-$ROOT/bindings/python/tests/test_version.py}"

# ---------------------------------------------------------------------------
# extractors — each echoes the version (or empty on no-match)
# ---------------------------------------------------------------------------

# First `^version = "X.Y.Z"` line (toml [package]/[workspace.package]/[project]).
toml_version() { # <file>
  grep -m1 -E '^version[[:space:]]*=' "$1" 2>/dev/null \
    | sed -nE 's/^version[[:space:]]*=[[:space:]]*"([^"]*)".*/\1/p'
}

# Compose MAJOR.MINOR.PATCH from `^pub const TST_VERSION_<PART> ... = N;`.
# Anchored on TST_VERSION_ to skip TST_ABI_VERSION_*.
rust_const_version() { # <file>
  local maj min pat
  maj="$(sed -nE 's/^pub const TST_VERSION_MAJOR[^=]*=[[:space:]]*([0-9]+).*/\1/p' "$1" 2>/dev/null | head -1)"
  min="$(sed -nE 's/^pub const TST_VERSION_MINOR[^=]*=[[:space:]]*([0-9]+).*/\1/p' "$1" 2>/dev/null | head -1)"
  pat="$(sed -nE 's/^pub const TST_VERSION_PATCH[^=]*=[[:space:]]*([0-9]+).*/\1/p' "$1" 2>/dev/null | head -1)"
  [ -n "$maj" ] && [ -n "$min" ] && [ -n "$pat" ] && printf '%s.%s.%s' "$maj" "$min" "$pat"
}

# Compose MAJOR.MINOR.PATCH from `^#define TST_VERSION_<PART> N`.
# Anchored on TST_VERSION_ to skip TST_ABI_VERSION_*.
cdefine_version() { # <file>
  local maj min pat
  maj="$(sed -nE 's/^#define TST_VERSION_MAJOR[[:space:]]+([0-9]+).*/\1/p' "$1" 2>/dev/null | head -1)"
  min="$(sed -nE 's/^#define TST_VERSION_MINOR[[:space:]]+([0-9]+).*/\1/p' "$1" 2>/dev/null | head -1)"
  pat="$(sed -nE 's/^#define TST_VERSION_PATCH[[:space:]]+([0-9]+).*/\1/p' "$1" 2>/dev/null | head -1)"
  [ -n "$maj" ] && [ -n "$min" ] && [ -n "$pat" ] && printf '%s.%s.%s' "$maj" "$min" "$pat"
}

# `assert tstrans.__version__ == "X.Y.Z"` -> X.Y.Z
pytest_version() { # <file>
  grep -m1 -E 'tstrans\.__version__[[:space:]]*==' "$1" 2>/dev/null \
    | sed -nE 's/.*==[[:space:]]*"([^"]*)".*/\1/p'
}

# ---------------------------------------------------------------------------
# run_check
# ---------------------------------------------------------------------------
run_check() { # [EXPECTED_VERSION]
  local expected="${1:-}"

  # If no explicit expected version, honour a v* tag ref.
  if [ -z "$expected" ] && [ -n "${GITHUB_REF_NAME:-}" ]; then
    case "$GITHUB_REF_NAME" in
      v[0-9]*) expected="${GITHUB_REF_NAME#v}" ;;
    esac
  fi

  # Parallel arrays of source-label and extracted version (no assoc arrays).
  local labels=() versions=()
  local v

  # `|| true` so an extractor that returns non-zero (e.g. a missing/reformatted
  # version line in rust_const_version/cdefine_version, which `&&`-chain to a
  # non-zero status) yields an EMPTY $v under `set -e` instead of aborting the
  # script with no diagnostic — the empty-check loop below then reports which
  # source failed. Still fails closed (rc=1) either way.
  v="$(toml_version "$RVC_CARGO_TOML" || true)"
  labels+=("workspace-cargo  ($RVC_CARGO_TOML)"); versions+=("$v")
  v="$(toml_version "$RVC_PYPROJECT" || true)"
  labels+=("pyproject        ($RVC_PYPROJECT)"); versions+=("$v")
  v="$(toml_version "$RVC_PY_CARGO" || true)"
  labels+=("tst-py-cargo     ($RVC_PY_CARGO)"); versions+=("$v")
  v="$(rust_const_version "$RVC_C_LIB" || true)"
  labels+=("c-version-consts ($RVC_C_LIB)"); versions+=("$v")
  v="$(cdefine_version "$RVC_C_HEADER" || true)"
  labels+=("c-header-defines ($RVC_C_HEADER)"); versions+=("$v")
  v="$(pytest_version "$RVC_PY_TEST" || true)"
  labels+=("python-version-test ($RVC_PY_TEST)"); versions+=("$v")

  # (1) every extraction must be non-empty.
  local i empty=0
  for i in "${!versions[@]}"; do
    if [ -z "${versions[$i]}" ]; then
      echo "FAIL: could not extract a version from: ${labels[$i]}" >&2
      echo "      (a file moved or its version-line format changed)" >&2
      empty=1
    fi
  done
  [ "$empty" -eq 0 ] || return 1

  # (2) all six mutually equal.
  local first="${versions[0]}" mismatch=0
  for i in "${!versions[@]}"; do
    [ "${versions[$i]}" = "$first" ] || mismatch=1
  done
  if [ "$mismatch" -ne 0 ]; then
    echo "FAIL: source versions disagree:" >&2
    for i in "${!versions[@]}"; do
      if [ "${versions[$i]}" = "$first" ]; then
        printf '  %-22s %s\n' "${versions[$i]}" "${labels[$i]}" >&2
      else
        printf '  %-22s %s   <-- differs\n' "${versions[$i]}" "${labels[$i]}" >&2
      fi
    done
    return 1
  fi

  # (3) optional expected/tag version must match the agreed version.
  if [ -n "$expected" ] && [ "$first" != "$expected" ]; then
    echo "FAIL: source version ($first) does not match expected/tag version ($expected)" >&2
    echo "      bump every source to $expected before tagging (or fix the tag)." >&2
    return 1
  fi

  # (4) success.
  if [ -n "$expected" ]; then
    echo "release-version-consistency: OK — all sources agree at $first (matches expected $expected)"
  else
    echo "release-version-consistency: OK — all sources agree at $first"
  fi
}

# ---------------------------------------------------------------------------
# self_test — drive the REAL script (recursively, via env overrides) against
# throwaway fixtures and assert each expected outcome.
# ---------------------------------------------------------------------------
self_test() {
  local tmp; tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' RETURN

  write_fixtures() { # <version>
    local ver="$1"
    local maj="${ver%%.*}" rest="${ver#*.}" min pat
    min="${rest%%.*}"; pat="${rest#*.}"

    printf '[workspace.package]\nversion      = "%s"\nrust-version = "1.85"\n' "$ver" > "$tmp/Cargo.toml"
    printf '[project]\nname    = "tstrans"\nversion = "%s"\n' "$ver" > "$tmp/pyproject.toml"
    printf '[package]\nname    = "tst-py"\nversion = "%s"\n' "$ver" > "$tmp/py-cargo.toml"
    # Include an ABI line to prove the TST_VERSION_ anchor skips TST_ABI_VERSION_.
    {
      printf 'pub const TST_VERSION_MAJOR: crate::c_types::c_int = %s;\n' "$maj"
      printf 'pub const TST_VERSION_MINOR: crate::c_types::c_int = %s;\n' "$min"
      printf 'pub const TST_VERSION_PATCH: crate::c_types::c_int = %s;\n' "$pat"
      printf 'pub const TST_ABI_VERSION_MINOR: crate::c_types::c_int = 13;\n'
    } > "$tmp/lib.rs"
    {
      printf '#define TST_ABI_VERSION_MINOR 13\n'
      printf '#define TST_VERSION_MAJOR %s\n' "$maj"
      printf '#define TST_VERSION_MINOR %s\n' "$min"
      printf '#define TST_VERSION_PATCH %s\n' "$pat"
    } > "$tmp/tstrans.h"
    printf '    assert tstrans.__version__ == "%s"\n' "$ver" > "$tmp/test_version.py"
  }

  # Run the real script with env pointed at the fixtures. $@ -> positional args.
  run_real() {
    RVC_CARGO_TOML="$tmp/Cargo.toml" \
    RVC_PYPROJECT="$tmp/pyproject.toml" \
    RVC_PY_CARGO="$tmp/py-cargo.toml" \
    RVC_C_LIB="$tmp/lib.rs" \
    RVC_C_HEADER="$tmp/tstrans.h" \
    RVC_PY_TEST="$tmp/test_version.py" \
    GITHUB_REF_NAME="" \
    bash "$0" "$@"
  }

  expect() { # <pass|fail> <label> [args...]
    local want="$1" label="$2"; shift 2
    local rc
    if run_real "$@" >/dev/null 2>&1; then rc=pass; else rc=fail; fi
    if [ "$rc" = "$want" ]; then
      echo "  ok: $label (expected $want)"
    else
      echo "  SELF-TEST FAIL: $label expected $want got $rc" >&2
      return 1
    fi
  }

  # (1) all six fixtures consistent -> pass
  write_fixtures "0.2.0"
  expect pass "all sources consistent" || return 1

  # (2) flip ONE fixture -> cross-source mismatch -> fail
  printf '    assert tstrans.__version__ == "0.1.0"\n' > "$tmp/test_version.py"
  expect fail "one source flipped to 0.1.0" || return 1

  # (3) consistent fixtures + matching expected arg -> pass
  write_fixtures "0.2.0"
  expect pass "expected arg 0.2.0 matches" 0.2.0 || return 1

  # (4) consistent fixtures + wrong expected arg -> tag mismatch -> fail
  expect fail "expected arg 9.9.9 mismatches" 9.9.9 || return 1

  echo "release-version-consistency self-test: OK"
}

# ---------------------------------------------------------------------------
# main
# ---------------------------------------------------------------------------
if [ "${1:-}" = "--self-test" ]; then
  self_test
else
  run_check "${1:-}"
fi
