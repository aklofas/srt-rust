#!/usr/bin/env bash
# Driver: run the `rust` rows of error-mapping.tsv through lib/coverage.sh.
# Each row asserts every <Enum>ErrorKind variant has an arm in its
# <proto>_error_to_code converter in crates/tst-c/src/error.rs.
set -uo pipefail
DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=lib/coverage.sh
. "$DIR/lib/coverage.sh"

TSV="$DIR/error-mapping.tsv"
ARM_FILE="crates/tst-c/src/error.rs"
while [[ $# -gt 0 ]]; do
    case "$1" in
        --tsv) TSV="$2"; shift 2 ;;
        --arm-file) ARM_FILE="$2"; shift 2 ;;
        *) echo "unknown arg: $1" >&2; exit 2 ;;
    esac
done
if [[ ! -f "$TSV" ]]; then echo "FAIL: TSV $TSV not found" >&2; exit 1; fi

rc=0
rows=0
while IFS=$'\t' read -r lang name enum src arm_fn; do
    [[ "$lang" == \#* || -z "${lang:-}" ]] && continue
    [[ "$lang" != "rust" ]] && continue
    arm_fn="${arm_fn%$'\r'}"  # tolerate a CRLF checkout (arm_fn is the last field)
    rows=$((rows + 1))
    if ! assert_rust_row "$enum" "$src" "$arm_fn" "$ARM_FILE"; then
        rc=1
    fi
done < "$TSV"

if [[ "$rows" -eq 0 ]]; then
    echo "FAIL: no rust rows found in $TSV (malformed table?)" >&2
    exit 1
fi
exit "$rc"
