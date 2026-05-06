#!/usr/bin/env bash
set -euo pipefail
# Regenerates AV1 test fixtures via ffmpeg + libaom-av1.
# Requires ffmpeg compiled with --enable-libaom (not in every distro build).
# Run locally; the produced .ts file is committed alongside this script
# (small enough at ~tens of KB that vendoring beats requiring libaom on every
# developer's machine).
cd "$(dirname "$0")"
ffmpeg -y -f lavfi -i testsrc=duration=2:size=320x240:rate=30 \
       -c:v libaom-av1 -cpu-used 8 -b:v 200k \
       -f mpegts lowres_av1.ts
echo "wrote $(pwd)/lowres_av1.ts"
