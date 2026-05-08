#!/usr/bin/env bash
# Regenerate synthetic audio-in-MPEG-TS fixtures for tst-core audio
# carriage tests. Each fixture is ~3 seconds of 440 Hz sine + a
# 320x240 H.264 baseline test pattern.
#
# Tools required (locally; not on CI): ffmpeg 6.x, tsduck 3.x.
# Verifies each output with ffprobe.
#
# Run: ./regen.sh

set -euo pipefail
cd "$(dirname "$0")"

A_INPUT=(-f lavfi -i 'sine=frequency=440:duration=3:sample_rate=44100')
V_INPUT=(-f lavfi -i 'testsrc=duration=3:size=320x240:rate=25')
V_ENC=(-c:v libx264 -profile:v baseline -tune zerolatency -pix_fmt yuv420p -g 25)

# ---- Conformant fixtures ----

ffmpeg -y -hide_banner -loglevel error \
  "${A_INPUT[@]}" "${V_INPUT[@]}" \
  "${V_ENC[@]}" \
  -c:a mp2 -b:a 192k \
  -f mpegts mp2.ts

ffmpeg -y -hide_banner -loglevel error \
  "${A_INPUT[@]}" "${V_INPUT[@]}" \
  "${V_ENC[@]}" \
  -c:a aac -b:a 128k \
  -f mpegts aac-adts.ts

# `-mpegts_flags +latm` is the magic: real LATM framing (ffprobe reports
# codec_name=aac_latm, distinct from codec_name=aac for ADTS).
ffmpeg -y -hide_banner -loglevel error \
  "${A_INPUT[@]}" "${V_INPUT[@]}" \
  "${V_ENC[@]}" \
  -c:a aac -b:a 128k \
  -mpegts_flags +latm \
  -f mpegts aac-latm.ts

ffmpeg -y -hide_banner -loglevel error \
  "${A_INPUT[@]}" "${V_INPUT[@]}" \
  "${V_ENC[@]}" \
  -c:a ac3 -b:a 192k \
  -f mpegts ac3.ts

# ---- Non-conformant rewrites (mimic real-world corpus quirks) ----

ffmpeg -y -hide_banner -loglevel error \
  "${A_INPUT[@]}" "${V_INPUT[@]}" \
  "${V_ENC[@]}" \
  -c:a libmp3lame -b:a 128k \
  -f mpegts mp3-conformant.ts

AUDIO_PID=$(ffprobe -v error -select_streams a -show_entries stream=id -of csv=p=0 mp3-conformant.ts | head -1)

# Shotover-ARS pattern: MP3 audio on user-private stream_type 0xF1
tsp -I file mp3-conformant.ts \
    -P pmt --remove-pid "${AUDIO_PID}" --add-pid "${AUDIO_PID}/0xF1" \
    -O file mp3-on-0xF1.ts 2>&1 | tail -3

# Corpus oddball: MP3 mislabeled as MPEG-1 Layer II (stream_type 0x03)
tsp -I file mp3-conformant.ts \
    -P pmt --remove-pid "${AUDIO_PID}" --add-pid "${AUDIO_PID}/0x03" \
    -O file mp3-on-0x03.ts 2>&1 | tail -3

# ---- Verify each fixture ----
echo
echo "=== verification ==="
for f in mp2.ts aac-adts.ts aac-latm.ts ac3.ts mp3-conformant.ts mp3-on-0xF1.ts mp3-on-0x03.ts; do
  size=$(stat -c%s "$f" 2>/dev/null || echo "MISSING")
  audio=$(ffprobe -v error -select_streams a -show_entries stream=codec_name,codec_tag -of csv=p=0 "$f" 2>/dev/null | head -1)
  printf '%-25s %8s bytes  audio=%s\n' "$f" "$size" "${audio:-NO_AUDIO}"
done
