#!/usr/bin/env bash
# Regenerate the H.264 / H.265 reference fixtures from FFmpeg encodes.
#
# Requires: ffmpeg with libx264 + libx265, python3.
# Usage: ./_regen.sh
# Writes RBSP bytes (NAL header stripped, Annex B prefix stripped,
# emulation prevention preserved) to ./h264/*.bin and ./h265/*.bin.

set -euo pipefail
cd "$(dirname "$0")"
mkdir -p h264 h265

FFOPTS="-hide_banner -loglevel error -y"

extract_nals() {
  python3 - "$@" << 'PYEOF'
import sys
codec, name, raw_path, out_dir = sys.argv[1:]
data = open(raw_path, 'rb').read()
nals, i = [], 0
while i < len(data) - 3:
    if data[i:i+4] == b'\x00\x00\x00\x01': start = i + 4
    elif data[i:i+3] == b'\x00\x00\x01':   start = i + 3
    else:
        i += 1; continue
    j = start
    while j < len(data) - 3:
        if data[j:j+4] == b'\x00\x00\x00\x01' or data[j:j+3] == b'\x00\x00\x01':
            break
        j += 1
    else: j = len(data)
    nals.append(data[start:j])
    i = j

if codec == 'h264':
    targets = {7: 'sps', 8: 'pps'}
    header_bytes = 1
else:  # h265
    targets = {32: 'vps', 33: 'sps', 34: 'pps'}
    header_bytes = 2

seen = set()
for nal in nals:
    nt = (nal[0] & 0x1F) if codec == 'h264' else ((nal[0] >> 1) & 0x3F)
    if nt in targets and nt not in seen:
        seen.add(nt)
        rbsp = nal[header_bytes:]
        kind = targets[nt]
        with open(f'{out_dir}/{name}_{kind}.bin', 'wb') as f:
            f.write(rbsp)
        print(f'  {kind} ({len(rbsp)}B)')
PYEOF
}

echo "H.264 1080p High@4.0 BT.709"
ffmpeg $FFOPTS -f lavfi -i color=c=black:s=1920x1080:r=30:d=0.2 \
  -c:v libx264 -profile:v high -level 4.0 -x264opts "keyint=1:no-scenecut" \
  -color_primaries bt709 -color_trc bt709 -colorspace bt709 \
  -f h264 /tmp/_h264_1.h264
extract_nals h264 h264_1080p_high40_bt709 /tmp/_h264_1.h264 h264

echo "H.264 720p Main@3.1"
ffmpeg $FFOPTS -f lavfi -i color=c=black:s=1280x720:r=30:d=0.2 \
  -c:v libx264 -profile:v main -level 3.1 -x264opts "keyint=1:no-scenecut" \
  -f h264 /tmp/_h264_2.h264
extract_nals h264 h264_720p_main31 /tmp/_h264_2.h264 h264

echo "H.265 1080p Main@4.0"
ffmpeg $FFOPTS -f lavfi -i color=c=black:s=1920x1080:r=30:d=0.2 \
  -c:v libx265 -preset ultrafast \
  -x265-params "keyint=1:no-scenecut:profile=main:level-idc=40" \
  -f hevc /tmp/_h265_1.h265
extract_nals h265 h265_1080p_main40 /tmp/_h265_1.h265 h265

echo "H.265 1080p Main10@5.0 PQ"
ffmpeg $FFOPTS -f lavfi -i color=c=black:s=1920x1080:r=30:d=0.2,format=yuv420p10le \
  -c:v libx265 -preset ultrafast \
  -x265-params "keyint=1:no-scenecut:profile=main10:level-idc=50:colorprim=bt2020:transfer=smpte2084:colormatrix=bt2020nc" \
  -f hevc /tmp/_h265_2.h265
extract_nals h265 h265_1080p_main10_50_pq /tmp/_h265_2.h265 h265

echo "Done. Inspect with: ls -l h264/*.bin h265/*.bin"
