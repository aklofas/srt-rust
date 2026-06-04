#!/usr/bin/env bash
# One-shot: print [[exempt]] blocks for every mappable public-api.txt item not
# already mapped in tests/coverage/surface-manifest.toml. Append its output to
# the manifest, then graduate items from exempt -> [[surface]] over time.
#
# Usage: bash scripts/gen/surface-exemptions.sh >> tests/coverage/surface-manifest.toml
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"; cd "$ROOT"
MANIFEST="tests/coverage/surface-manifest.toml"
CRATES="rist-sys tst-core tst-pipeline tst-rist tst-rtp tst-srt tst-tcp tst-udp"

mapped="$(mktemp)"; trap 'rm -f "$mapped"' EXIT
grep -E '^item = ' "$MANIFEST" 2>/dev/null | sed -E 's/^item = "(.*)"/\1/' | sort -u > "$mapped"

extract_items() {  # stdin: a public-api.txt; stdout: one canonical item key per mappable line
  awk '
    /^pub (const fn|fn|struct|enum|trait|type|const) / {
      line=$0
      sub(/^pub const fn /, "pub fn ", line)   # normalize "pub const fn" -> "pub fn"
      sub(/^pub (fn|struct|enum|trait|type|const) /, "", line)
      sub(/[(<].*$/, "", line)                 # drop fn args / generics
      sub(/[[:space:]]*=.*$/, "", line)        # drop type alias RHS: " = ..."
      sub(/:[[:space:]].*$/, "", line)         # drop ": Type" annotation (const/assoc-type)
      sub(/[[:space:]]+$/, "", line)
      if (line != "") print line
    }
  '
}

for c in $CRATES; do
  f="crates/$c/public-api.txt"; [ -f "$f" ] || continue
  extract_items < "$f"
done | sort -u | while IFS= read -r item; do
  grep -Fxq "$item" "$mapped" && continue
  printf '[[exempt]]\nitem = "%s"\nreason = "bulk bootstrap 2026-06-03; graduate to [[surface]] as coverage is asserted"\n\n' "$item"
done
