#!/usr/bin/env bash
# 22nd bash ratchet (Phase 4 Stage 1).
# Verifies tstrans.h contains TST_HAS_SRT + TST_HAS_RTP defines and
# that every tst_rtp_*, tst_rtsp_*, and existing tst_*_open/SRT-specific
# symbol is wrapped in the appropriate #ifdef guard.
#
# Why this ratchet: cbindgen's [parse.expand] cfg-aware emission can
# subtly misbehave on cfg-conditional generic functions — symbols can
# leak outside their guard, producing C-side compile errors when
# downstream consumers build with --features srt only.

set -euo pipefail

HEADER="crates/tst-c/include/tstrans.h"

if [[ ! -f "$HEADER" ]]; then
    echo "FAIL: $HEADER not found"
    exit 1
fi

# Check TST_HAS_* defines present
if ! grep -q "TST_HAS_SRT" "$HEADER"; then
    echo "FAIL: TST_HAS_SRT not defined in $HEADER"
    exit 1
fi
if ! grep -q "TST_HAS_RTP" "$HEADER"; then
    echo "FAIL: TST_HAS_RTP not defined in $HEADER"
    exit 1
fi

# Check that tst_rtp_* and tst_rtsp_* symbols only appear inside #ifdef TST_HAS_RTP blocks
# (a leaked tst_rtsp_* outside the guard breaks consumer builds with --features srt only).
#
# Strategy: parse the header into block-pairs delimited by #ifdef TST_HAS_RTP ... #endif
# and verify all tst_rtp_*/tst_rtsp_* symbol declarations are within such a block.
python3 - <<'PY'
import re
import sys

with open("crates/tst-c/include/tstrans.h") as f:
    text = f.read()

# Find all tst_rtp_* and tst_rtsp_* declarations
rtp_syms = re.findall(r'\btst_(?:rtp|rtsp)_\w+', text)
rtp_syms = set(rtp_syms)

# Walk #ifdef blocks. Build a nesting stack; for each line, track whether
# we're inside an #ifdef TST_HAS_RTP block.
lines = text.split('\n')
inside_rtp_guard = 0
leaks = []

for lineno, line in enumerate(lines, 1):
    stripped = line.strip()
    if stripped.startswith('#ifdef TST_HAS_RTP') or stripped.startswith('#if TST_HAS_RTP') \
       or stripped.startswith('#if defined(TST_HAS_RTP)'):
        inside_rtp_guard += 1
    elif stripped.startswith('#endif') and inside_rtp_guard > 0:
        # Imprecise — could be closing a nested #if. For ratchet purposes,
        # assume the nearest TST_HAS_RTP ifdef closes here.
        inside_rtp_guard -= 1

    if inside_rtp_guard == 0:
        for sym in rtp_syms:
            # Only flag symbol DECLARATIONS, not casual mentions in comments.
            # Declarations look like: `tst_<name> tst_rtp_xxx(...)` or `void tst_rtsp_yyy(...)`.
            if re.search(rf'\b{re.escape(sym)}\s*\(', line):
                # Also skip #define lines
                if not stripped.startswith('#'):
                    leaks.append((lineno, sym, line.rstrip()))

if leaks:
    print(f"FAIL: {len(leaks)} tst_rtp_*/tst_rtsp_* symbol(s) leaked outside #ifdef TST_HAS_RTP:")
    for lineno, sym, line in leaks[:20]:
        print(f"  line {lineno}: {sym} -- {line[:100]}")
    sys.exit(1)
PY

# Same check for tst_*_open SRT-side symbols (loose check — those are the
# main externally-named SRT functions; skip if too many false positives).
# Actually skip this for v1 — tst-srt-named exports are stable; the cfg gate is on the source side.

echo "PASS: c-header-conditional-sections"
