#!/usr/bin/env bash
# Transport-axis interop matrix: exchange the `tst-interop` driver's
# synthetic MPEG-TS/KLV traffic with real third-party tools (ffmpeg,
# TSDuck's `tsp`, GStreamer, VLC) over live SRT/RIST/UDP/TCP/HLS/RTSP
# sessions on this box, and write one evidence JSON file per cell.
#
# linux-x86_64 only (see lib.sh's header for the shell-portability
# stance this whole directory takes). Requires `jq` and `python3`
# (port allocation) on PATH; peer tools are individually optional —
# `have()` gates every cell that needs one, emitting
# SKIPPED_TOOL_MISSING instead of a fake pass/fail when it's absent.
#
# Usage:
#   run-matrix.sh --outdir DIR [--seconds N] [--cells GLOB] [--profiles LIST]
#
#   --outdir DIR      required. Cell JSON -> DIR/cells/*.json, per-cell
#                      combined (us + peer) logs -> DIR/logs/*.log,
#                      merged report -> DIR/results.json + results.md.
#   --seconds N        stream duration per cell, a positive integer
#                      (default 10). Every cell's process-kill budget
#                      scales off this (see lib.sh's cell_timeout).
#   --cells GLOB       bash-glob filter over cell ids (default "*", i.e.
#                      every cell). Cell ids look like
#                      "<transport>/<direction>-<peer>[-encrypted]",
#                      e.g. "srt/us-to-ffmpeg" — pass 'srt/*' to run
#                      just the SRT block. Quote it so the shell doesn't
#                      expand the glob itself.
#   --profiles LIST    comma-separated profile names (default
#                      "baseline"). Every cell in the matrix runs once
#                      per listed profile. The transport-axis cell table
#                      this script implements was designed against
#                      "baseline" only — the format axis (multiple
#                      profiles) is a later task's job; this flag exists
#                      because it's part of the documented interface,
#                      not because more than one profile is exercised
#                      today.
#
# Exit code: `tst-interop report merge`'s (0 iff every FAIL matched an
# expectations.toml entry — see that file's header for the grammar).
# scripts/interop/expectations.toml ships empty; run-matrix.sh alone
# will therefore exit 1 on ANY genuine FAIL — that's Task 12's job to
# triage into real expectations rows, not this script's.
#
# Cell naming / tier semantics: see README.md in this directory.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
# shellcheck source=lib.sh
source "$SCRIPT_DIR/lib.sh"

OUTDIR=""
SECONDS_ARG=10
CELLS_GLOB="*"
PROFILES_ARG="baseline"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --outdir)
      OUTDIR=$2
      shift 2
      ;;
    --seconds)
      SECONDS_ARG=$2
      shift 2
      ;;
    --cells)
      CELLS_GLOB=$2
      shift 2
      ;;
    --profiles)
      PROFILES_ARG=$2
      shift 2
      ;;
    -h | --help)
      sed -n '2,40p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      echo "run-matrix.sh: unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

[[ -n "$OUTDIR" ]] || {
  echo "run-matrix.sh: --outdir is required" >&2
  exit 2
}
[[ "$SECONDS_ARG" =~ ^[0-9]+$ && "$SECONDS_ARG" -gt 0 ]] || {
  echo "run-matrix.sh: --seconds must be a positive integer, got: $SECONDS_ARG" >&2
  exit 2
}

mkdir -p "$OUTDIR/cells" "$OUTDIR/logs" "$OUTDIR/work"
CELLS_DIR="$OUTDIR/cells"
LOGS_DIR="$OUTDIR/logs"
WORK="$OUTDIR/work"

# Settle time between binding/starting the listening side of a cell and
# starting its peer — every scheme here binds/listens near-instantly
# once its process starts (confirmed for SRT/RIST/TCP/UDP/HLS/RTSP
# while developing this script), so a short fixed sleep is enough; no
# scheme needs the port to be independently probed as "ready".
SETTLE=2

# AES passphrase shared by every "encrypted" cell below. Length must
# clear SRT's 10-byte floor (RFC/libsrt: 10..79 chars); arbitrary
# otherwise — this is test-fixture traffic, not a real secret.
ENCRYPTION_PASSPHRASE="tst-interop-matrix-fixture-secret"

echo "run-matrix: building tst-interop (release)..." >&2
(cd "$REPO_ROOT" && SRT_FORCE_VENDORED=1 cargo build --release -p tst-interop)
BIN="$REPO_ROOT/target/release/tst-interop"

jq -n \
  --arg host "$(hostname)" \
  --arg date "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --arg seconds "$SECONDS_ARG" \
  --arg cells_glob "$CELLS_GLOB" \
  --argjson tools "$(tool_versions_json)" \
  '{host: $host, date: $date, seconds_per_cell: ($seconds | tonumber),
    cells_glob: $cells_glob, tools: $tools}' \
  >"$OUTDIR/meta.json"

# ---------------------------------------------------------------------
# Cell shapes
# ---------------------------------------------------------------------
# Every shape below writes ONE `$CELLS_DIR/<slug>.json` (via lib.sh's
# emit_* helpers) and ONE `$LOGS_DIR/<slug>.log` combining both sides'
# output. `direction` in the emitted cell always names OUR role
# (`send` = we push/serve, `recv` = we pull/listen) — matches
# crates/tst-interop/src/report.rs's own test fixtures.

# run_send_peer_recv <id> <peer> <tier> <our_url> <out_file> -- <peer_cmd...>
#
# We push (caller/connect) over <our_url>; the peer listens/receives
# and writes <out_file>. Verdict: our send must exit 0, <out_file> must
# be nonempty, and `verify --file <out_file>` must pass; a "transparent"
# tier additionally requires the sent-side stream_sha256 to equal
# <out_file>'s.
run_send_peer_recv() {
  local id=$1 peer=$2 tier=$3 our_url=$4 out_file=$5
  shift 5
  [[ "${1:-}" == "--" ]] && shift
  local -a peer_cmd=("$@")

  cell_selected "$id" || return 0
  if ! have "$peer"; then
    emit_skipped "$id" "$peer" send "$tier" "$peer not installed on this box"
    return 0
  fi

  local log="$LOGS_DIR/$(slug "$id").log"
  : >"$log"
  local budget
  budget=$(cell_timeout "$SECONDS_ARG")

  {
    echo "=== cell: $id (tier=$tier, profile=$PROFILE) ==="
    echo "--- peer ($peer), listening/receiving -> $out_file ---"
    printf '+ '
    printf '%q ' "${peer_cmd[@]}"
    echo
  } >>"$log"

  timeout --kill-after=5 "${budget}s" "${peer_cmd[@]}" >>"$log" 2>&1 &
  local peer_pid=$!
  sleep "$SETTLE"

  local send_json="$WORK/$(slug "$id")-send.json"
  echo "--- us: send ($our_url) ---" >>"$log"
  local send_rc=0
  timeout --kill-after=3 "${budget}s" \
    "$BIN" send --profile "$PROFILE" --url "$our_url" --seconds "$SECONDS_ARG" --json "$send_json" \
    >>"$log" 2>&1 || send_rc=$?

  local peer_rc=0
  wait "$peer_pid" || peer_rc=$?
  echo "--- exit codes: send=$send_rc peer=$peer_rc ---" >>"$log"

  if [[ $send_rc -ne 0 ]]; then
    emit_fail "$id" "$peer" send "$tier" "$log" - "our send failed (exit $send_rc) — see log"
    return 0
  fi
  if [[ ! -s "$out_file" ]]; then
    emit_fail "$id" "$peer" send "$tier" "$log" - "peer produced no (or empty) output file (peer exit $peer_rc)"
    return 0
  fi

  local vjson="$WORK/$(slug "$id")-verify.json"
  local vrc=0
  "$BIN" verify --file "$out_file" --expect "$PROFILE" --seconds "$SECONDS_ARG" --json "$vjson" \
    >>"$log" 2>&1 || vrc=$?

  local -a reasons=()
  if [[ $vrc -ne 0 ]]; then
    reasons+=("verify FAILed against the peer's captured file: $(jq -r '.failures | join("; ")' "$vjson" 2>/dev/null)")
  fi
  if [[ "$tier" == "transparent" ]]; then
    local sent_hash got_hash
    sent_hash=$(jq -r '.stream_sha256' "$send_json")
    got_hash=$(jq -r '.metrics.stream_sha256' "$vjson")
    if [[ "$sent_hash" != "$got_hash" ]]; then
      reasons+=("byte-transparent tier: stream_sha256 mismatch (sent $sent_hash, peer capture $got_hash)")
    fi
  fi

  local mjson="$WORK/$(slug "$id")-metrics.json"
  metrics_only "$vjson" "$mjson"
  if [[ ${#reasons[@]} -eq 0 ]]; then
    emit_pass "$id" "$peer" send "$tier" "$log" "$mjson"
  else
    emit_fail "$id" "$peer" send "$tier" "$log" "$mjson" "${reasons[@]}"
  fi
}

# run_peer_send_recv <id> <peer> <tier> <our_url> -- <peer_cmd...>
#
# The peer pushes (from $GEN_FILE, already baked into peer_cmd by the
# caller); we listen/receive over <our_url>. Verdict: our recv must
# produce a JSON report and PASS its own profile-invariant check; a
# "transparent" tier additionally requires our received stream_sha256
# to equal $GEN_FILE's own (pre-computed in $GEN_STREAM_SHA).
run_peer_send_recv() {
  local id=$1 peer=$2 tier=$3 our_url=$4
  shift 4
  [[ "${1:-}" == "--" ]] && shift
  local -a peer_cmd=("$@")

  cell_selected "$id" || return 0
  if ! have "$peer"; then
    emit_skipped "$id" "$peer" recv "$tier" "$peer not installed on this box"
    return 0
  fi

  local log="$LOGS_DIR/$(slug "$id").log"
  : >"$log"
  local budget
  budget=$(cell_timeout "$SECONDS_ARG")

  echo "=== cell: $id (tier=$tier, profile=$PROFILE) ===" >>"$log"
  echo "--- us: recv (listening on $our_url) ---" >>"$log"

  local recv_json="$WORK/$(slug "$id")-recv.json"
  timeout --kill-after=5 "${budget}s" \
    "$BIN" recv --url "$our_url" --expect "$PROFILE" --seconds "$SECONDS_ARG" --json "$recv_json" \
    >>"$log" 2>&1 &
  local recv_pid=$!
  sleep "$SETTLE"

  {
    echo "--- peer ($peer), sending from $GEN_FILE ---"
    printf '+ '
    printf '%q ' "${peer_cmd[@]}"
    echo
  } >>"$log"
  local peer_rc=0
  timeout --kill-after=5 "${budget}s" "${peer_cmd[@]}" >>"$log" 2>&1 || peer_rc=$?

  local recv_rc=0
  wait "$recv_pid" || recv_rc=$?
  echo "--- exit codes: peer=$peer_rc recv=$recv_rc ---" >>"$log"

  if [[ ! -s "$recv_json" ]]; then
    emit_fail "$id" "$peer" recv "$tier" "$log" - \
      "our recv produced no JSON output (recv exit $recv_rc, peer exit $peer_rc)"
    return 0
  fi

  local pass
  pass=$(jq -r '.pass' "$recv_json")
  local -a reasons=()
  if [[ "$pass" != "true" ]]; then
    reasons+=("recv FAILed: $(jq -r '.failures | join("; ")' "$recv_json")")
  fi
  if [[ "$tier" == "transparent" ]]; then
    local got_hash
    got_hash=$(jq -r '.metrics.stream_sha256' "$recv_json")
    if [[ "$got_hash" != "$GEN_STREAM_SHA" ]]; then
      reasons+=("byte-transparent tier: stream_sha256 mismatch (source $GEN_STREAM_SHA, received $got_hash)")
    fi
  fi

  local mjson="$WORK/$(slug "$id")-metrics.json"
  metrics_only "$recv_json" "$mjson"
  if [[ ${#reasons[@]} -eq 0 ]]; then
    emit_pass "$id" "$peer" recv "$tier" "$log" "$mjson"
  else
    emit_fail "$id" "$peer" recv "$tier" "$log" "$mjson" "${reasons[@]}"
  fi
}

# run_serve_peer_pull <id> <peer> <tier> <our_serve_url> <out_file> -- <peer_cmd...>
#
# We BIND (HLS HTTP server / RTSP server) and serve $SECONDS_ARG of
# traffic, wall-clock paced, then keep serving for the driver's own
# LINGER (10s — crates/tst-interop/src/serve.rs); the peer pulls and
# writes <out_file>. Same file-based verdict shape as
# run_send_peer_recv, just no sent-side CellMetrics (serve mode's
# `send` doesn't accept --json — see serve.rs's doc comment) and a
# longer timeout budget for our own side.
run_serve_peer_pull() {
  local id=$1 peer=$2 tier=$3 our_serve_url=$4 out_file=$5
  shift 5
  [[ "${1:-}" == "--" ]] && shift
  local -a peer_cmd=("$@")

  cell_selected "$id" || return 0
  if ! have "$peer"; then
    emit_skipped "$id" "$peer" send "$tier" "$peer not installed on this box"
    return 0
  fi

  local log="$LOGS_DIR/$(slug "$id").log"
  : >"$log"
  local sbudget pbudget
  sbudget=$(serve_timeout "$SECONDS_ARG")
  pbudget=$(cell_timeout "$SECONDS_ARG")

  echo "=== cell: $id (tier=$tier, profile=$PROFILE) ===" >>"$log"
  echo "--- us: serve ($our_serve_url) ---" >>"$log"

  timeout --kill-after=5 "${sbudget}s" \
    "$BIN" send --profile "$PROFILE" --url "$our_serve_url" --seconds "$SECONDS_ARG" \
    >>"$log" 2>&1 &
  local serve_pid=$!
  sleep 2 # bind + mux/HTTP-server setup

  {
    echo "--- peer ($peer), pulling -> $out_file ---"
    printf '+ '
    printf '%q ' "${peer_cmd[@]}"
    echo
  } >>"$log"
  local peer_rc=0
  timeout --kill-after=5 "${pbudget}s" "${peer_cmd[@]}" >>"$log" 2>&1 || peer_rc=$?

  local serve_rc=0
  wait "$serve_pid" || serve_rc=$?
  echo "--- exit codes: peer=$peer_rc serve=$serve_rc ---" >>"$log"

  if [[ ! -s "$out_file" ]]; then
    emit_fail "$id" "$peer" send "$tier" "$log" - \
      "peer produced no (or empty) output file (peer exit $peer_rc, serve exit $serve_rc)"
    return 0
  fi

  local vjson="$WORK/$(slug "$id")-verify.json"
  local vrc=0
  "$BIN" verify --file "$out_file" --expect "$PROFILE" --seconds "$SECONDS_ARG" --json "$vjson" \
    >>"$log" 2>&1 || vrc=$?

  local mjson="$WORK/$(slug "$id")-metrics.json"
  metrics_only "$vjson" "$mjson"
  if [[ $vrc -eq 0 ]]; then
    emit_pass "$id" "$peer" send "$tier" "$log" "$mjson"
  else
    emit_fail "$id" "$peer" send "$tier" "$log" "$mjson" \
      "verify FAILed against the peer's captured file: $(jq -r '.failures | join("; ")' "$vjson" 2>/dev/null)"
  fi
}

# run_serve_peer_probe <id> <peer> <our_serve_url> -- <peer_cmd...>
#
# Same serve setup as run_serve_peer_pull, but the peer is a
# decode-only probe (no capture file to verify) — mirrors
# release-validation.sh's "Decoder compatibility matrix" step: PASS iff
# the peer's combined output has no error/fatal/cannot/failed marker.
# Tier is always "n/a" (there is nothing byte- or structure-level to
# compare — a clean decode is the only signal).
run_serve_peer_probe() {
  local id=$1 peer=$2 our_serve_url=$3
  shift 3
  [[ "${1:-}" == "--" ]] && shift
  local -a peer_cmd=("$@")

  cell_selected "$id" || return 0
  if ! have "$peer"; then
    emit_skipped "$id" "$peer" send n/a "$peer not installed on this box"
    return 0
  fi

  local log="$LOGS_DIR/$(slug "$id").log"
  : >"$log"
  local sbudget pbudget
  sbudget=$(serve_timeout "$SECONDS_ARG")
  pbudget=$(cell_timeout "$SECONDS_ARG")

  echo "=== cell: $id (decode-probe, no file-verify, profile=$PROFILE) ===" >>"$log"
  echo "--- us: serve ($our_serve_url) ---" >>"$log"

  timeout --kill-after=5 "${sbudget}s" \
    "$BIN" send --profile "$PROFILE" --url "$our_serve_url" --seconds "$SECONDS_ARG" \
    >>"$log" 2>&1 &
  local serve_pid=$!
  sleep 2

  local probe_log="$WORK/$(slug "$id")-probe.log"
  {
    echo "--- peer ($peer), decode-probe ---"
    printf '+ '
    printf '%q ' "${peer_cmd[@]}"
    echo
  } >>"$log"
  local peer_rc=0
  timeout --kill-after=5 "${pbudget}s" "${peer_cmd[@]}" >"$probe_log" 2>&1 || peer_rc=$?
  cat "$probe_log" >>"$log"

  local serve_rc=0
  wait "$serve_pid" || serve_rc=$?
  echo "--- exit codes: probe=$peer_rc serve=$serve_rc ---" >>"$log"

  # Same permissive, case-insensitive, whole-word marker grep
  # release-validation.sh's decoder-compatibility step uses — but first
  # strip VLC's known-harmless sandbox-startup noise (no PulseAudio/
  # D-Bus session/$DISPLAY in this headless box: VLC's default
  # `--extraintf` list tries `dbus`+`globalhotkeys` regardless of
  # `--intf dummy`, logs an "error"-labeled line when each fails to
  # init, then falls back to the dummy interface successfully — none of
  # that reflects on whether the RTSP stream itself decoded cleanly).
  # A real stream/decode problem (e.g. VLC's own "buffer deadlock
  # prevented") is untouched by this filter.
  local filtered
  filtered=$(grep -Ev 'vlcpulse audio output error|dbus interface error|globalhotkeys|no suitable interface module|main libvlc error: interface' \
    "$probe_log")
  if grep -Eqi '\b(error|fatal|cannot|failed)\b' <<<"$filtered"; then
    local sample
    sample=$(grep -Ei '\b(error|fatal|cannot|failed)\b' <<<"$filtered" | head -3 | tr '\n' ';')
    emit_fail "$id" "$peer" send n/a "$log" - "decoder reported error/fatal markers: $sample"
  else
    emit_pass "$id" "$peer" send n/a "$log" -
  fi
}

# ---------------------------------------------------------------------
# Per-transport cell definitions
# ---------------------------------------------------------------------
# Every function below assumes $GEN_FILE / $GEN_STREAM_SHA are set
# (the per-profile loop at the bottom of this script does that before
# calling any of them) and reads $PROFILE / $SECONDS_ARG / $BIN.

srt_cells() {
  local port

  port=$(free_port)
  run_send_peer_recv "srt/us-to-ffmpeg" ffmpeg remux \
    "srt://127.0.0.1:$port?mode=caller" "$WORK/srt_us-to-ffmpeg.ts" -- \
    ffmpeg -y -loglevel warning -i "srt://127.0.0.1:$port?mode=listener" \
    -map 0 -c copy -copy_unknown -f mpegts "$WORK/srt_us-to-ffmpeg.ts"

  port=$(free_port)
  run_peer_send_recv "srt/ffmpeg-to-us" ffmpeg remux \
    "srt://:$port?mode=listener" -- \
    ffmpeg -y -re -loglevel warning -i "$GEN_FILE" -c copy -copy_unknown -f mpegts \
    "srt://127.0.0.1:$port?mode=caller"

  port=$(free_port)
  run_send_peer_recv "srt/us-to-tsp" tsp transparent \
    "srt://127.0.0.1:$port?mode=caller" "$WORK/srt_us-to-tsp.ts" -- \
    tsp -I srt --listener ":$port" -O file "$WORK/srt_us-to-tsp.ts"

  port=$(free_port)
  run_peer_send_recv "srt/tsp-to-us" tsp transparent \
    "srt://:$port?mode=listener" -- \
    tsp -I file "$GEN_FILE" -P regulate -O srt --caller "127.0.0.1:$port" --linger 5

  port=$(free_port)
  run_send_peer_recv "srt/us-to-gst" gst-launch-1.0 transparent \
    "srt://127.0.0.1:$port?mode=caller" "$WORK/srt_us-to-gst.ts" -- \
    gst-launch-1.0 srtsrc "uri=srt://0.0.0.0:$port?mode=listener" ! \
    filesink "location=$WORK/srt_us-to-gst.ts"

  port=$(free_port)
  run_peer_send_recv "srt/gst-to-us" gst-launch-1.0 transparent \
    "srt://:$port?mode=listener" -- \
    gst-launch-1.0 filesrc "location=$GEN_FILE" ! \
    tsparse set-timestamps=true smoothing-latency=100000 ! \
    srtsink "uri=srt://127.0.0.1:$port?mode=caller"

  # Encrypted (ffmpeg + tsp pairs only, per the plan — gst's srtsrc/
  # srtsink encryption story wasn't part of this arc's scope).
  port=$(free_port)
  run_send_peer_recv "srt/us-to-ffmpeg-encrypted" ffmpeg remux \
    "srt://127.0.0.1:$port?mode=caller&passphrase=$ENCRYPTION_PASSPHRASE" \
    "$WORK/srt_us-to-ffmpeg-encrypted.ts" -- \
    ffmpeg -y -loglevel warning -passphrase "$ENCRYPTION_PASSPHRASE" \
    -i "srt://127.0.0.1:$port?mode=listener" \
    -map 0 -c copy -copy_unknown -f mpegts "$WORK/srt_us-to-ffmpeg-encrypted.ts"

  # -passphrase is an SRT-protocol AVOption: it must sit between -i and
  # the OUTPUT url here (the encrypted side is the SRT *output*, not
  # the plain-file input) — ffmpeg applies an option to whichever -i/
  # output it immediately precedes, and rejects it as "not found" on
  # the wrong side (confirmed empirically while developing this script).
  port=$(free_port)
  run_peer_send_recv "srt/ffmpeg-to-us-encrypted" ffmpeg remux \
    "srt://:$port?mode=listener&passphrase=$ENCRYPTION_PASSPHRASE" -- \
    ffmpeg -y -re -loglevel warning \
    -i "$GEN_FILE" -c copy -copy_unknown -passphrase "$ENCRYPTION_PASSPHRASE" -f mpegts \
    "srt://127.0.0.1:$port?mode=caller"

  port=$(free_port)
  run_send_peer_recv "srt/us-to-tsp-encrypted" tsp transparent \
    "srt://127.0.0.1:$port?mode=caller&passphrase=$ENCRYPTION_PASSPHRASE" \
    "$WORK/srt_us-to-tsp-encrypted.ts" -- \
    tsp -I srt --listener ":$port" --passphrase "$ENCRYPTION_PASSPHRASE" \
    -O file "$WORK/srt_us-to-tsp-encrypted.ts"

  port=$(free_port)
  run_peer_send_recv "srt/tsp-to-us-encrypted" tsp transparent \
    "srt://:$port?mode=listener&passphrase=$ENCRYPTION_PASSPHRASE" -- \
    tsp -I file "$GEN_FILE" -P regulate -O srt --caller "127.0.0.1:$port" \
    --linger 5 --passphrase "$ENCRYPTION_PASSPHRASE"
}

udp_cells() {
  local port

  port=$(free_port)
  run_send_peer_recv "udp/us-to-ffmpeg" ffmpeg remux \
    "udp://127.0.0.1:$port" "$WORK/udp_us-to-ffmpeg.ts" -- \
    ffmpeg -y -loglevel warning -i "udp://127.0.0.1:$port" \
    -map 0 -c copy -copy_unknown -f mpegts "$WORK/udp_us-to-ffmpeg.ts"

  port=$(free_port)
  run_peer_send_recv "udp/ffmpeg-to-us" ffmpeg remux \
    "udp://127.0.0.1:$port" -- \
    ffmpeg -y -re -loglevel warning -i "$GEN_FILE" -c copy -copy_unknown -f mpegts \
    "udp://127.0.0.1:$port"

  port=$(free_port)
  run_send_peer_recv "udp/us-to-tsp" tsp transparent \
    "udp://127.0.0.1:$port" "$WORK/udp_us-to-tsp.ts" -- \
    tsp -I ip "$port" -O file "$WORK/udp_us-to-tsp.ts"

  port=$(free_port)
  run_peer_send_recv "udp/tsp-to-us" tsp transparent \
    "udp://127.0.0.1:$port" -- \
    tsp -I file "$GEN_FILE" -P regulate -O ip "127.0.0.1:$port"
}

rist_cells() {
  local port

  port=$(free_port)
  run_send_peer_recv "rist/us-to-ffmpeg" ffmpeg remux \
    "rist://127.0.0.1:$port" "$WORK/rist_us-to-ffmpeg.ts" -- \
    ffmpeg -y -loglevel warning -i "rist://@0.0.0.0:$port" \
    -map 0 -c copy -copy_unknown -f mpegts "$WORK/rist_us-to-ffmpeg.ts"

  port=$(free_port)
  run_peer_send_recv "rist/ffmpeg-to-us" ffmpeg remux \
    "rist://@0.0.0.0:$port" -- \
    ffmpeg -y -re -loglevel warning -i "$GEN_FILE" -c copy -copy_unknown -f mpegts \
    "rist://127.0.0.1:$port"

  port=$(free_port)
  run_send_peer_recv "rist/us-to-tsp" tsp transparent \
    "rist://127.0.0.1:$port" "$WORK/rist_us-to-tsp.ts" -- \
    tsp -I rist "rist://@0.0.0.0:$port" -O file "$WORK/rist_us-to-tsp.ts"

  port=$(free_port)
  run_peer_send_recv "rist/tsp-to-us" tsp transparent \
    "rist://@0.0.0.0:$port" -- \
    tsp -I file "$GEN_FILE" -P regulate -O rist "rist://127.0.0.1:$port"
}

tcp_cells() {
  local port

  # ffmpeg listens (`?listen=1`); we connect as the caller.
  port=$(free_port)
  run_send_peer_recv "tcp/us-to-ffmpeg" ffmpeg remux \
    "tcp://127.0.0.1:$port" "$WORK/tcp_us-to-ffmpeg.ts" -- \
    ffmpeg -y -loglevel warning -i "tcp://0.0.0.0:$port?listen=1" \
    -map 0 -c copy -copy_unknown -f mpegts "$WORK/tcp_us-to-ffmpeg.ts"

  # We listen; ffmpeg connects as a plain (non-listen) caller.
  port=$(free_port)
  run_peer_send_recv "tcp/ffmpeg-to-us" ffmpeg remux \
    "tcp://0.0.0.0:$port?listen=1" -- \
    ffmpeg -y -re -loglevel warning -i "$GEN_FILE" -c copy -copy_unknown -f mpegts \
    "tcp://127.0.0.1:$port"
}

hls_cells() {
  local port

  port=$(free_port)
  run_serve_peer_pull "hls/ffmpeg-pull" ffmpeg remux \
    "hls://127.0.0.1:$port" "$WORK/hls_ffmpeg-pull.ts" -- \
    ffmpeg -y -loglevel warning -i "http://127.0.0.1:$port/playlist.m3u8" \
    -map 0 -c copy -copy_unknown -f mpegts "$WORK/hls_ffmpeg-pull.ts"

  port=$(free_port)
  run_serve_peer_pull "hls/tsp-pull" tsp remux \
    "hls://127.0.0.1:$port" "$WORK/hls_tsp-pull.ts" -- \
    tsp -I hls "http://127.0.0.1:$port/playlist.m3u8" -O file "$WORK/hls_tsp-pull.ts"
}

rtsp_cells() {
  local port

  port=$(free_port)
  run_serve_peer_pull "rtsp-serve/ffmpeg-pull" ffmpeg remux \
    "rtsp://127.0.0.1:$port/mount" "$WORK/rtsp-serve_ffmpeg-pull.ts" -- \
    ffmpeg -y -loglevel warning -rtsp_transport tcp \
    -i "rtsp://127.0.0.1:$port/mount" \
    -map 0 -c copy -copy_unknown -f mpegts "$WORK/rtsp-serve_ffmpeg-pull.ts"

  port=$(free_port)
  run_serve_peer_probe "rtsp-serve/vlc-probe" cvlc \
    "rtsp://127.0.0.1:$port/mount" -- \
    cvlc --intf dummy --play-and-exit --no-audio \
    "--run-time=$SECONDS_ARG" "rtsp://127.0.0.1:$port/mount"

  # Best-effort: peer-to-peer only (vlc serves, ffmpeg consumes) — our
  # own `recv`/transport::make_recv has no rtsp:// connect-side support
  # (see transport.rs's scheme dispatch: srt/udp/tcp/rist only), so
  # there is no way for tst-interop itself to be the RTSP client here.
  # "us" only brackets this cell (gen the source file / verify the
  # capture); expect flakiness (VLC's --sout RTSP serving is fiddly) —
  # wired as `known_flaky` in expectations.toml starting Task 12.
  cell_selected "rtsp-consume/vlc-serve-ffmpeg-pull" || return 0
  if ! have cvlc || ! have ffmpeg; then
    local missing="cvlc and/or ffmpeg"
    emit_skipped "rtsp-consume/vlc-serve-ffmpeg-pull" "vlc+ffmpeg" n/a remux \
      "$missing not installed on this box"
    return 0
  fi
  port=$(free_port)
  local log="$LOGS_DIR/$(slug "rtsp-consume/vlc-serve-ffmpeg-pull").log"
  : >"$log"
  local budget
  budget=$(cell_timeout "$SECONDS_ARG")
  {
    echo "=== cell: rtsp-consume/vlc-serve-ffmpeg-pull (tier=remux, profile=$PROFILE) ==="
    echo "--- peer: vlc serves \$GEN_FILE over RTSP ---"
  } >>"$log"
  timeout --kill-after=5 "${budget}s" \
    cvlc "$GEN_FILE" --intf dummy --sout "#rtp{sdp=rtsp://:$port/s}" \
    >>"$log" 2>&1 &
  local vlc_pid=$!
  sleep "$SETTLE"

  local out="$WORK/rtsp-consume_vlc-serve-ffmpeg-pull.ts"
  echo "--- peer: ffmpeg pulls rtsp://127.0.0.1:$port/s -> $out ---" >>"$log"
  local ff_rc=0
  timeout --kill-after=5 "${budget}s" \
    ffmpeg -y -loglevel warning -rtsp_transport tcp -i "rtsp://127.0.0.1:$port/s" \
    -map 0 -c copy -copy_unknown -f mpegts "$out" \
    >>"$log" 2>&1 || ff_rc=$?

  local vlc_rc=0
  wait "$vlc_pid" || vlc_rc=$?
  echo "--- exit codes: ffmpeg=$ff_rc vlc=$vlc_rc ---" >>"$log"

  if [[ ! -s "$out" ]]; then
    emit_fail "rtsp-consume/vlc-serve-ffmpeg-pull" "vlc+ffmpeg" n/a remux "$log" - \
      "ffmpeg produced no (or empty) capture (ffmpeg exit $ff_rc, vlc exit $vlc_rc)"
    return 0
  fi
  local vjson="$WORK/rtsp-consume_vlc-serve-ffmpeg-pull-verify.json"
  local vrc=0
  "$BIN" verify --file "$out" --expect "$PROFILE" --seconds "$SECONDS_ARG" --json "$vjson" \
    >>"$log" 2>&1 || vrc=$?
  local mjson="$WORK/rtsp-consume_vlc-serve-ffmpeg-pull-metrics.json"
  metrics_only "$vjson" "$mjson"
  if [[ $vrc -eq 0 ]]; then
    emit_pass "rtsp-consume/vlc-serve-ffmpeg-pull" "vlc+ffmpeg" n/a remux "$log" "$mjson"
  else
    emit_fail "rtsp-consume/vlc-serve-ffmpeg-pull" "vlc+ffmpeg" n/a remux "$log" "$mjson" \
      "verify FAILed against ffmpeg's captured file: $(jq -r '.failures | join("; ")' "$vjson" 2>/dev/null)"
  fi
}

# ---------------------------------------------------------------------
# Run every axis, once per --profiles entry
# ---------------------------------------------------------------------

IFS=',' read -r -a PROFILE_LIST <<<"$PROFILES_ARG"
for PROFILE in "${PROFILE_LIST[@]}"; do
  export PROFILE
  echo "run-matrix: profile=$PROFILE seconds=$SECONDS_ARG cells=$CELLS_GLOB" >&2

  GEN_FILE="$WORK/gensrc-$PROFILE.ts"
  "$BIN" gen --profile "$PROFILE" --seconds "$SECONDS_ARG" --out "$GEN_FILE"
  GEN_VERIFY_JSON="$WORK/gensrc-$PROFILE-verify.json"
  "$BIN" verify --file "$GEN_FILE" --expect "$PROFILE" --seconds "$SECONDS_ARG" --json "$GEN_VERIFY_JSON"
  GEN_STREAM_SHA=$(jq -r '.metrics.stream_sha256' "$GEN_VERIFY_JSON")
  export GEN_FILE GEN_STREAM_SHA

  srt_cells
  udp_cells
  rist_cells
  tcp_cells
  hls_cells
  rtsp_cells
done

# ---------------------------------------------------------------------
# Merge + render
# ---------------------------------------------------------------------

echo "run-matrix: merging cell results..." >&2
merge_rc=0
"$BIN" report merge \
  --cells-dir "$CELLS_DIR" \
  --expectations "$SCRIPT_DIR/expectations.toml" \
  --meta "$OUTDIR/meta.json" \
  --out "$OUTDIR/results.json" || merge_rc=$?

"$BIN" report render --in "$OUTDIR/results.json" --out "$OUTDIR/results.md"

echo "run-matrix: wrote $OUTDIR/results.json + $OUTDIR/results.md (exit $merge_rc)" >&2
exit "$merge_rc"
