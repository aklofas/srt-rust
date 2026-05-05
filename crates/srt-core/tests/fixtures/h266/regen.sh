#!/usr/bin/env bash
set -euo pipefail
# Regenerates H.266 test fixtures via ffmpeg + libvvenc.
# Requires ffmpeg compiled with --enable-libvvenc (not in most distro builds).
# Run locally; the produced .ts file is committed alongside this script
# (small enough at ~tens of KB that vendoring beats requiring vvenc on every
# developer's machine).
cd "$(dirname "$0")"
ffmpeg -y -f lavfi -i testsrc=duration=2:size=320x240:rate=30 \
       -c:v libvvenc -preset faster -b:v 200k \
       -f mpegts lowres_h266.ts
echo "wrote $(pwd)/lowres_h266.ts"
