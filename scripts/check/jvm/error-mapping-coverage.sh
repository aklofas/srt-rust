#!/usr/bin/env bash
# tst-jni error-mapping coverage: every <Family>Exception.Kind constant must have
# a throw_<family>(env, "<CONST>", ...) call site in bindings/jvm/src/. Mirrors
# scripts/check/python/error-mapping-coverage.sh. Tighten the grep when the row
# graduates to a [[surface]] manifest entry.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
TSV="$ROOT/scripts/ratchets/error-mapping.tsv"
SRC="$ROOT/bindings/jvm/src"
JAVA="$ROOT/bindings/jvm/src/main/java/org/tstrans"
fail=0
while IFS=$'\t' read -r lang name cls makefn _rest; do
  [[ "$lang" == \#* || -z "${lang:-}" ]] && continue
  [ "$lang" = "java" ] || continue
  family="${cls%.Kind}"                      # e.g. DemuxException
  jfile="$JAVA/$family.java"
  if [ ! -f "$jfile" ]; then echo "FAIL[$name]: missing $jfile"; fail=1; continue; fi
  # extract enum constants from `enum Kind { A, B, ... }`. Strip // line comments
  # and inline /* */ comments first so an UPPER_CASE token in a comment can
  # neither poison (false FAIL) nor vacuously satisfy the check.
  consts=$(sed -n '/enum Kind/,/}/p' "$jfile" \
    | sed 's@//.*@@; s@/\*.*\*/@@g' \
    | grep -oE '[A-Z][A-Z0-9_]+' | grep -v '^Kind$' || true)
  if [ -z "$consts" ]; then
    echo "FAIL[$name]: no Kind constants found in $jfile"
    fail=1
    continue
  fi
  for c in $consts; do
    if ! grep -rqE "${makefn}\s*\(\s*[^,]*,\s*\"$c\"" "$SRC"; then
      echo "FAIL[$name]: $family.Kind.$c has no ${makefn}(env, \"$c\", ...) call site"
      fail=1
    fi
  done
done < "$TSV"
[ "$fail" -eq 0 ] && echo "jvm error-mapping coverage: OK"
exit "$fail"
