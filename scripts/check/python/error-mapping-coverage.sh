#!/usr/bin/env bash
# Python error-mapping coverage (consolidated). Replaces the former
# per-protocol check-py-{udp,tcp,rist,rtp,rtsp,srt,hls}-error-mapping-coverage.sh
# clones; rows live in scripts/ratchets/error-mapping.tsv. Lives under
# scripts/check/ so the local pre-push loop (every *.sh under scripts/check/)
# and the explicit CI step both pick it up.
set -euo pipefail
exec "$(dirname "$0")/../../ratchets/run-py-coverage.sh"
