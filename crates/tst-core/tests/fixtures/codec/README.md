# Codec parameter-set test fixtures

Each `.bin` file is one parameter-set NAL's RBSP body — the bytes after
the codec's NAL header (1 byte for H.264, 2 bytes for H.265) and after
any Annex B start code. Emulation prevention bytes (`00 00 03`) are
preserved, matching what `NalUnit::H264 { payload }` and
`NalUnit::H265 { payload }` ship from the demuxer.

Each fixture's expected parsed values come from the encoder parameters:

| Fixture | Resolution | Profile | Level | Bit depth | Chroma | Color |
|---|---|---|---|---|---|---|
| `h264/h264_1080p_high40_bt709_*` | 1920×1080 | High (100) | 4.0 (40) | 8 | 4:2:0 | BT.709 |
| `h264/h264_720p_main31_*` | 1280×720 | Main (77) | 3.1 (31) | 8 | 4:2:0 | (default) |
| `h265/h265_1080p_main40_*` | 1920×1080 | Main (1) | 4.0 (120) | 8 | 4:2:0 | (default) |
| `h265/h265_1080p_main10_50_pq_*` | 1920×1080 | Main 10 (2) | 5.0 (150) | 10 | 4:2:0 | BT.2020 + PQ |

Note H.265 `general_level_idc` is in units of 30 × level (so level 4.0
is `level_idc=120`, level 5.0 is `level_idc=150`).

To regenerate any of these fixtures, run `./_regen.sh` in this
directory. The script extracts SPS/PPS (and VPS for H.265) RBSP
bytes from FFmpeg-produced encoded streams.
