#!/usr/bin/env bash
# Driver: run the `py` rows of error-mapping.tsv through lib/coverage.sh.
# Each row asserts every `class <Enum>ErrorKind` member in
# tstrans.exceptions has a make_<proto>_error call site under bindings/python/src/.
set -uo pipefail
DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$DIR/../.." && pwd)"          # scripts/ratchets -> scripts -> workspace root
# shellcheck source=lib/coverage.sh
. "$DIR/lib/coverage.sh"

TSV="$DIR/error-mapping.tsv"
EXC_FILE="$ROOT/bindings/python/python/tstrans/exceptions.py"
SRC_DIR="$ROOT/bindings/python/src"
while [[ $# -gt 0 ]]; do
    case "$1" in
        --tsv) TSV="$2"; shift 2 ;;
        --exc-file) EXC_FILE="$2"; shift 2 ;;
        --src-dir) SRC_DIR="$2"; shift 2 ;;
        *) echo "unknown arg: $1" >&2; exit 2 ;;
    esac
done
if [[ ! -f "$TSV" ]]; then echo "FAIL: TSV $TSV not found" >&2; exit 1; fi

rc=0
rows=0
while IFS=$'\t' read -r lang name class make_fn _rest; do
    [[ "$lang" == \#* || -z "${lang:-}" ]] && continue
    [[ "$lang" != "py" ]] && continue
    rows=$((rows + 1))
    if ! assert_py_row "$class" "$make_fn" "$EXC_FILE" "$SRC_DIR"; then
        rc=1
    fi
done < "$TSV"

if [[ "$rows" -eq 0 ]]; then
    echo "FAIL: no py rows found in $TSV (malformed table?)" >&2
    exit 1
fi
exit "$rc"
