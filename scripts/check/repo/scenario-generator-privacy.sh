#!/usr/bin/env bash
# Ratchet: the cross-binding scenario generator is synthetic-only. It must never
# read from testfiles/, any local/ dir, or an absolute corpus path. We scan the
# generator sources with comments stripped (the doc comments legitimately say
# "NEVER reads from testfiles/local/"), so only real code is checked.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
cd "$ROOT"
fail=0
err() { echo "FAIL: $*" >&2; fail=1; }

# Files in scope: the binary + every scenario source.
mapfile -t files < <(printf '%s\n' \
  crates/tst-integration/src/bin/gen_scenarios.rs \
  $(git ls-files 'crates/tst-integration/src/scenarios/*.rs'))

# Forbidden tokens in *code* (post comment-strip).
pattern='testfiles|fixtures/local|/local/|\.\./testfiles|/home/|CARGO_HOME'

for f in "${files[@]}"; do
  [[ -f "$f" ]] || { err "scan target missing: $f"; continue; }
  # Strip Rust line/doc comments (everything from // to EOL), then grep.
  hits="$(sed -E 's://.*$::' "$f" | grep -nE "$pattern" || true)"
  if [[ -n "$hits" ]]; then
    err "private-corpus access pattern in generator code: $f"
    printf '%s\n' "$hits" | sed 's/^/    /' >&2
  fi
done

[[ "$fail" -eq 0 ]] && echo "PASS: scenario-generator-privacy (synthetic-only)"
exit "$fail"
