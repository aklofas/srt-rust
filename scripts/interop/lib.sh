#!/usr/bin/env bash
# Shared helpers for the transport-axis interop orchestrator
# (run-matrix.sh). Meant to be SOURCED, not executed.
#
# linux-x86_64 only, like the rest of this arc's live-tool matrix — the
# peer tools (ffmpeg/tsp/gst-launch-1.0/vlc/cvlc) are discovered at
# runtime via `have()` rather than assumed, but the shell itself is not
# made macOS-portable (bash arrays, `mapfile`-style idioms, and GNU
# `timeout --kill-after` are all fair game; see scripts/check/**'s
# opposite convention — this directory is intentionally NOT part of
# that rail sweep). No bare `sleep N` as a standalone statement inside a
# loop (the sandbox blocks that shape); the short fixed settle sleeps
# below are one-shot, not loop bodies.
#
# NOT sourced with `set -e` baked in on purpose: the whole point of this
# file's callers is to keep going after one cell's peer command fails.
# Every external command a caller cares about the exit code of must use
# the `cmd || rc=$?` form (never a bare `cmd` on its own line followed by
# reading `$?`) so a failure doesn't blow up the calling script instead.
set -uo pipefail

# ---------------------------------------------------------------------
# Tool discovery
# ---------------------------------------------------------------------

have() { command -v "$1" >/dev/null 2>&1; }

# First line of `$1 $2`'s output (every peer tool below prints its
# version banner on line 1, to stdout or stderr depending on the tool).
_tool_version_line() {
  "$1" "$2" 2>&1 | head -1
}

# JSON object of {tool: "version string"} for every peer tool this
# matrix knows about that's actually installed on this box. A missing
# tool is simply an absent key (not a null value) — keeps the --meta
# blob small and lets a reader tell "not installed" apart from "we
# failed to read its version" at a glance.
tool_versions_json() {
  local json='{}'
  local spec name flag v
  for spec in ffmpeg:-version tsp:--version gst-launch-1.0:--version vlc:--version mpv:--version; do
    name=${spec%%:*}
    flag=${spec##*:}
    if have "$name"; then
      v=$(_tool_version_line "$name" "$flag")
      json=$(jq --arg k "$name" --arg v "$v" '. + {($k): $v}' <<<"$json")
    fi
  done
  printf '%s' "$json"
}

# ---------------------------------------------------------------------
# Port allocation
# ---------------------------------------------------------------------

# Ask the OS for an unused loopback port via a throwaway UDP bind —
# mirrors crates/tst-interop/tests/loopback.rs's own `free_port()`
# (small TOCTOU race between this probe's close and the real bind that
# follows; same accepted trade-off documented there).
free_port() {
  python3 -c 'import socket; s=socket.socket(socket.AF_INET, socket.SOCK_DGRAM); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1]); s.close()'
}

# ---------------------------------------------------------------------
# Cell-id <-> filesystem-safe name
# ---------------------------------------------------------------------

# Cell ids use `/` (e.g. "srt/us-to-ffmpeg"); filenames can't. One
# escape for every log/work/cell-json filename this orchestrator writes.
slug() {
  printf '%s' "${1//\//_}"
}

# ---------------------------------------------------------------------
# Cell selection (--cells GLOB)
# ---------------------------------------------------------------------

# True iff cell id "$1" matches the orchestrator's --cells glob
# (default "*", i.e. everything). A plain bash `case` glob — supports
# the same `*`/`?`/`[...]` shapes run-matrix.sh's own --help documents.
cell_selected() {
  case "$1" in
    $CELLS_GLOB) return 0 ;;
    *) return 1 ;;
  esac
}

# ---------------------------------------------------------------------
# Timeout budgets
# ---------------------------------------------------------------------

# Wall-clock multiplier + flat floor applied to a cell's --seconds
# duration to get the `timeout` budget for ONE side of a cell (our
# process or the peer's): generous enough to absorb connection setup
# (SRT/RIST handshake, DNS-free loopback TCP connect) without being so
# loose that a genuinely wedged peer stalls the whole matrix for
# minutes. See `serve_timeout` for the HLS/RTSP-serve variant, which
# additionally has to outlive the driver's own 10s post-push LINGER
# (crates/tst-interop/src/serve.rs's `LINGER` const).
CELL_TIMEOUT_MULTIPLIER=3
CELL_TIMEOUT_FLOOR=15
# Extra margin added on top of `cell_timeout` for the side of a cell
# that binds/serves (HLS/RTSP `send --url hls://...` / `rtsp://...`):
# covers the driver's own LINGER (10s) plus mux/HTTP-server setup slop.
SERVE_LINGER_MARGIN=20

# `$1` = --seconds value for this cell (a plain non-negative integer —
# run-matrix.sh's own --seconds parsing enforces that, see its --help).
cell_timeout() {
  echo $(( $1 * CELL_TIMEOUT_MULTIPLIER + CELL_TIMEOUT_FLOOR ))
}

serve_timeout() {
  echo $(( $(cell_timeout "$1") + SERVE_LINGER_MARGIN ))
}

# Budget for operations that aren't tied to a cell's --seconds duration at
# all (the bootstrap `gen`/`verify` of the per-profile source file, and the
# final `report merge`/`report render`) but still must never hang the whole
# matrix unattended (CI/soak runs — Tasks 13/14 — depend on that). Every
# one of these operates on data this task's own findings show can arrive
# malformed from a third-party remuxer (`verify`) or, for `report
# merge`/`render`, on N small already-written JSON files — none of them
# have a legitimate reason to run long, so one generous flat floor covers
# all of them rather than scaling a multiplier that doesn't apply.
REPORT_TIMEOUT=120

# ---------------------------------------------------------------------
# Cell JSON emission (RawCell — see crates/tst-interop/src/report.rs)
# ---------------------------------------------------------------------
#
# Callers set these globals before sourcing this file's emit_* helpers:
#   CELLS_DIR  - directory RawCell JSON files are written into
#   OUTDIR     - the run's top-level --outdir (logs are recorded
#                relative to this, for a portable results.json)
#   PROFILE    - the profile name every cell in the current loop
#                iteration is testing (see run-matrix.sh's per-profile
#                loop)

# emit_cell <id> <peer> <direction> <tier> <verdict> <logfile> \
#           <metrics_json_file|-> [failure_text...]
#
# The one place that actually writes a `$CELLS_DIR/<slug>.json` file.
# `metrics_json_file` is a path to a file holding one JSON CellMetrics
# object (e.g. what `tst-interop send/verify --json FILE` wrote), or the
# literal `-` for `metrics: null` (SKIPPED_TOOL_MISSING, or a FAIL where
# neither side ever produced a parseable capture). Every remaining
# argument is one human-readable failure string.
emit_cell() {
  local id=$1 peer=$2 direction=$3 tier=$4 verdict=$5 logfile=$6 metrics_file=$7
  shift 7
  local failures_json='[]'
  if [[ $# -gt 0 ]]; then
    failures_json=$(printf '%s\n' "$@" | jq -R . | jq -s .)
  fi
  local metrics_json='null'
  if [[ "$metrics_file" != "-" && -s "$metrics_file" ]]; then
    metrics_json=$(cat "$metrics_file")
  fi
  local log_rel=$logfile
  if [[ "$logfile" == "$OUTDIR"/* ]]; then
    log_rel=${logfile#"$OUTDIR"/}
  fi
  local out_file="$CELLS_DIR/$(slug "$id").json"
  jq -n \
    --arg id "$id" --arg profile "$PROFILE" --arg peer "$peer" \
    --arg direction "$direction" --arg tier "$tier" --arg verdict "$verdict" \
    --arg log "$log_rel" --argjson failures "$failures_json" --argjson metrics "$metrics_json" \
    '{id: $id, profile: $profile, peer: $peer, direction: $direction, tier: $tier,
      verdict: $verdict, failures: $failures, metrics: $metrics, log: $log}' \
    >"$out_file"
}

emit_pass() {
  local id=$1 peer=$2 direction=$3 tier=$4 logfile=$5 metrics_file=$6
  emit_cell "$id" "$peer" "$direction" "$tier" PASS "$logfile" "$metrics_file"
}

emit_fail() {
  local id=$1 peer=$2 direction=$3 tier=$4 logfile=$5 metrics_file=$6
  shift 6
  emit_cell "$id" "$peer" "$direction" "$tier" FAIL "$logfile" "$metrics_file" "$@"
}

# metrics_only <verify_report_file> <out_file>
#
# `tst-interop recv|verify --json` both write a VerifyReport
# (`{pass, failures, metrics: {...}}`) — one level of nesting deeper
# than `send --json`'s bare CellMetrics, and deeper than what
# `emit_cell`'s `metrics_file` argument expects (a bare CellMetrics
# object, matching RawCell.metrics's shape in report.rs). Callers that
# only have a VerifyReport file (every `recv`/`verify` call site) must
# route it through this first — passing a VerifyReport straight to
# emit_cell embeds the wrong shape and `report merge` fails to parse
# the cell (missing `video_aus` etc., since it lands one level too
# high).
# Never fails (always returns 0), even if `$1` is missing/truncated/
# malformed JSON — every one of its 4 call sites in run-matrix.sh is a
# bare statement (`metrics_only "$vjson" "$mjson"`), and this function's
# own exit status IS that bare statement's exit status under `set -e`;
# a real-world truncated VerifyReport (e.g. a peer or `verify` process
# killed mid-write by a `timeout --kill-after`) would otherwise abort the
# whole matrix run right here. On failure `$2` ends up empty (the `>`
# redirection still creates/truncates it before jq runs and fails) —
# `emit_cell`'s own `-s "$metrics_file"` check already treats an empty
# file the same as "no metrics" (`metrics: null`), so no caller-side
# fallback is needed beyond never dying.
metrics_only() {
  jq '.metrics' "$1" >"$2" 2>/dev/null || true
}

emit_skipped() {
  local id=$1 peer=$2 direction=$3 tier=$4 reason=$5
  local logfile="$LOGS_DIR/$(slug "$id").log"
  printf 'SKIPPED_TOOL_MISSING: %s\n' "$reason" >"$logfile"
  emit_cell "$id" "$peer" "$direction" "$tier" SKIPPED_TOOL_MISSING "$logfile" - "$reason"
}
