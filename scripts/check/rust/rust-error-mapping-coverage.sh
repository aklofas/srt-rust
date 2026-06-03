#!/usr/bin/env bash
# Rust error-mapping coverage (consolidated). Replaces the former per-protocol
# check-{udp,tcp,rist,hls}-error-mapping-coverage.sh clones; rows live in
# scripts/ratchets/error-mapping.tsv. Lives under scripts/check/ so the local
# pre-push loop and CI pick it up like every other ratchet.
set -euo pipefail
exec "$(dirname "$0")/../../ratchets/run-rust-coverage.sh"
