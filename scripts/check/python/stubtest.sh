#!/usr/bin/env bash
# Verifies the core tst-py .pyi stubs (io/codec/klv/mpegts) match the live
# runtime surface via `mypy stubtest`. Build-dependent (needs a maturin-built
# tstrans + mypy in bindings/python/.venv), so it is EXCLUDED from the bare
# `find scripts/check` pre-push sweep's hard-fail contract: when the venv /
# mypy / built module are absent it prints SKIP and exits 0. CI runs it for
# real in the `python-core` job right after `maturin develop --release`.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
VENV="$ROOT/bindings/python/.venv"
ALLOWLIST="$ROOT/scripts/ratchets/stubtest-allowlist.txt"
MODULES=(tstrans.io tstrans.codec tstrans.klv tstrans.mpegts)

PY="$VENV/bin/python"
if [ ! -x "$PY" ] || ! "$PY" -c 'import mypy, tstrans' >/dev/null 2>&1; then
  echo "SKIP: bindings/python/.venv missing mypy or a built tstrans"
  echo "      (run: cd bindings/python && maturin develop --release && pip install mypy)"
  exit 0
fi

"$PY" -m mypy.stubtest "${MODULES[@]}" --allowlist "$ALLOWLIST"
echo "stubtest: tstrans.{io,codec,klv,mpegts} OK"
