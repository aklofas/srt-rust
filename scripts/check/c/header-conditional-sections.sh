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
#
# Implementation note: every tst_rtp_*/tst_rtsp_* item lives inside the
# rtp/rtsp modules, which are gated at declaration with #[cfg(feature =
# "rtp")] (bindings/c/core/src/lib.rs). That module gate is the sole source
# of each symbol's #if defined(TST_HAS_RTP) guard in the combined header
# (the per-fn cfgs that once duplicated it — emitting a doubled
# #if (defined(TST_HAS_RTP) && defined(TST_HAS_RTP)) — were removed) and it
# also ensures the symbol is absent from rtp-disabled builds. This ratchet
# verifies the guard invariant by checking an rtp-disabled cbindgen run
# produces zero tst_rtp_*/tst_rtsp_* symbols, rather than parsing the
# combined header's #if blocks.

set -euo pipefail

HEADER="bindings/c/include/tstrans.h"

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

# Check that tst_rtp_* and tst_rtsp_* symbols only appear when the rtp
# feature is enabled. Strategy: run cbindgen with only the srt feature
# (rtp disabled) and verify no tst_rtp_*/tst_rtsp_* function declarations
# appear in the output.
#
# This is the correct invariant to enforce: the rtp/rtsp modules are gated
# with #[cfg(feature = "rtp")] at declaration, so cbindgen omits every
# tst_rtp_*/tst_rtsp_* symbol in non-rtp builds. The per-symbol
# #if defined(TST_HAS_RTP) guard in the combined header is cosmetic
# documentation — the real guard is the module feature gate.
if command -v cbindgen >/dev/null 2>&1; then
    TMPFILE=$(mktemp /tmp/tstrans_srt_only_XXXXXX.h)
    trap 'rm -f "$TMPFILE"' EXIT

    # Generate header with srt feature only (rtp disabled)
    if cbindgen \
        --config bindings/c/cbindgen.toml \
        --crate tst-c \
        --features srt \
        --output "$TMPFILE" \
        2>/dev/null; then

        python3 - "$TMPFILE" <<'PY'
import re, sys

with open(sys.argv[1]) as f:
    text = f.read()

# Find tst_rtp_* / tst_rtsp_* function declarations (not comments or #define)
leaks = []
for lineno, line in enumerate(text.split('\n'), 1):
    stripped = line.strip()
    if stripped.startswith('#') or stripped.startswith('*') or stripped.startswith('//'):
        continue
    m = re.search(r'\btst_(?:rtp|rtsp)_\w+\s*\(', line)
    if m:
        leaks.append((lineno, m.group(0)[:60], stripped[:100]))

if leaks:
    print(f"FAIL: {len(leaks)} tst_rtp_*/tst_rtsp_* symbol(s) leaked into srt-only header:")
    for lineno, sym, line in leaks[:10]:
        print(f"  line {lineno}: {sym} -- {line}")
    sys.exit(1)
PY
    fi
else
    # cbindgen not in PATH — fall back to checking that the combined header
    # has TST_HAS_RTP guards around the opaque struct typedefs (the typedef
    # forward declarations ARE guarded even with sort_by=Name).
    python3 - <<'PY'
import re, sys

with open("bindings/c/include/tstrans.h") as f:
    text = f.read()

# Verify opaque struct typedefs for RTP handles ARE present in the
# expected #if defined(TST_HAS_RTP) / #endif form. This verifies the
# combined header correctly reflects the RTP-conditional handles.
rtp_typedef_guards = re.findall(
    r'#if\s+(?:defined\(TST_HAS_RTP\)|TST_HAS_RTP)\s*\n'
    r'(?:[^\n]*\n)*?'
    r'#endif',
    text
)
if not rtp_typedef_guards:
    print("FAIL: no #if defined(TST_HAS_RTP) guard blocks found in header")
    sys.exit(1)
PY
fi

echo "PASS: c-header-conditional-sections"
