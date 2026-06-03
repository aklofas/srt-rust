#!/usr/bin/env bash
# 12th bash ratchet: every CodecParseError variant in tst-core must appear
# in tst-py/src/errors.rs `codec_parse_error_to_pyerr` mapper.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
RUST_FILE="$ROOT/crates/tst-core/src/codec/mod.rs"
PY_FILE="$ROOT/bindings/python/src/errors.rs"

# Extract every variant name from `pub enum CodecParseError { ... }` block.
variants=$(awk '
  /^pub enum CodecParseError/ { in_block = 1; next }
  in_block && /^}/ { in_block = 0 }
  in_block && /^    [A-Z][A-Za-z0-9]*[ ({]/ {
    sub(/^    /, "")
    sub(/[ {(].*$/, "")
    print
  }
' "$RUST_FILE")

missing=()
for v in $variants; do
  if ! grep -q "CodecParseError::$v" "$PY_FILE"; then
    missing+=("$v")
  fi
done

if [ "${#missing[@]}" -ne 0 ]; then
  echo "ERROR: CodecParseError variants missing from $PY_FILE:" >&2
  for v in "${missing[@]}"; do echo "  - $v" >&2; done
  exit 1
fi

echo "OK: all $(echo "$variants" | wc -l | tr -d ' ') CodecParseError variants mapped"
