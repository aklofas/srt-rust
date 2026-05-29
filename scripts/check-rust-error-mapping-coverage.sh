#!/usr/bin/env bash
# Rust error-mapping coverage (consolidated). Replaces the former per-protocol
# check-{udp,tcp,rist,hls}-error-mapping-coverage.sh clones; rows live in
# scripts/ratchets/error-mapping.tsv. Kept as scripts/check-*.sh so the local
# pre-push glob and CI pick it up like every other ratchet.
set -euo pipefail
exec "$(dirname "$0")/ratchets/run-rust-coverage.sh"
