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

# No-producer constants: present for cross-binding enum parity with tst-py but
# with NO throw site. tst-py's DemuxErrorKind carries the same dead constant;
# py's rail tolerates it structurally (it checks Rust-variant->arm, never
# constant->producer). Keyed "<Family>.<CONST>".
is_no_producer_exempt() {
  case "$1" in
    "DemuxException.UNEXPECTED_EOF") return 0 ;;
    *) return 1 ;;
  esac
}

while IFS=$'\t' read -r lang name cls makefn _rest; do
  [[ "$lang" == \#* || -z "${lang:-}" ]] && continue
  [ "$lang" = "java" ] || continue
  family="${cls%.Kind}"                      # e.g. DemuxException
  jfile="$JAVA/$family.java"
  if [ ! -f "$jfile" ]; then echo "FAIL[$name]: missing $jfile"; fail=1; continue; fi
  # extract enum constants from `enum Kind { A, B, ... }`. Strip ALL comments
  # FIRST (whole file), in order: single-line /* */ (incl. /** */) via substitution,
  # then any remaining multi-line /* ... */ block via a range delete, then //
  # line comments. This must precede the `enum Kind`..`}` range selection — a
  # per-constant Javadoc's inline {@code ...} carries a stray `}` that would
  # otherwise truncate the range mid-enum and silently drop later constants.
  consts=$(sed 's@/\*.*\*/@@g' "$jfile" \
    | sed '\@/\*@,\@\*/@d' \
    | sed 's@//.*@@' \
    | sed -n '/enum Kind/,/}/p' \
    | grep -oE '[A-Z][A-Z0-9_]+' | grep -v '^Kind$' || true)
  if [ -z "$consts" ]; then
    echo "FAIL[$name]: no Kind constants found in $jfile"
    fail=1
    continue
  fi
  for c in $consts; do
    if is_no_producer_exempt "$family.$c"; then continue; fi
    if ! grep -rqE "${makefn}\s*\(\s*[^,]*,\s*\"$c\"" "$SRC"; then
      echo "FAIL[$name]: $family.Kind.$c has no ${makefn}(env, \"$c\", ...) call site"
      fail=1
    fi
  done
done < "$TSV"
[ "$fail" -eq 0 ] && echo "jvm error-mapping coverage: OK"
exit "$fail"
