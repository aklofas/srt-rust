#!/usr/bin/env bash
# Verify that libtstrans.so's dynamic export table contains zero
# srt_*/SRT_* symbols. Plan B's build.rs symbol-hygiene wiring (Linux:
# -Wl,--exclude-libs=ALL; macOS: -Wl,-exported_symbols_list,exports.txt)
# restricts exports to tst_*/TST_*; this ratchet enforces the result at
# CI time.
#
# A regression here means either:
#   - the build.rs link-arg was dropped from emission
#   - libsrt has a new C-visible symbol the version-script didn't
#     anticipate (extremely unlikely — --exclude-libs=ALL is total)
#   - someone added a libsrt re-export to tst-c without going through
#     the Rust srt_sys::* binding path
#
# All three are real bugs.

set -euo pipefail

cd "$(dirname "$0")/.."

# Linux GNU-only (matches symbol_audit.rs test gating).
if [ "$(uname -s)" != "Linux" ]; then
    echo "  SKIP: non-Linux host (this ratchet uses GNU nm)"
    exit 0
fi

LIB="target/debug/libtstrans.so"
if [ ! -f "$LIB" ]; then
    echo "Building tst-c first to produce $LIB..."
    SRT_FORCE_VENDORED=1 cargo build -p tst-c
fi

LEAKED=$(nm -D -g --defined-only "$LIB" 2>&1 \
    | awk '$2 ~ /^[TDR]$/ { print $3 }' \
    | { grep -E '^(srt_|SRT_)' || true; })

if [ -n "$LEAKED" ]; then
    echo "FAIL: libtstrans.so dynamic export table contains srt_*/SRT_* symbols:"
    echo "$LEAKED" | sed 's/^/  /'
    echo
    echo "Plan B's symbol-hygiene wiring (bindings/c/build.rs) should hide these."
    echo "Diagnose:"
    echo "  cargo build -p tst-c -v 2>&1 | grep exclude-libs"
    exit 1
fi

echo "OK: libtstrans.so exports zero srt_*/SRT_* symbols"
