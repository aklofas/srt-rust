# RTSP client empirical interop matrix

Manual best-effort. Run with `RTSP_INTEROP_<TARGET>=1` env var.
Fill rows as targets are verified.

| Target | UDP | TCP-interleaved | Basic auth | Digest MD5 | Digest SHA-256 | rtsps:// | Notes |
|---|---|---|---|---|---|---|---|
| ffmpeg RTSP server | ☐ | ☐ | ☐ | ☐ | ☐ | ☐ | `ffmpeg -re -i x.ts -c copy -f rtsp rtsp://0.0.0.0:8554/test` |
| MediaMTX | ☐ | ☐ | ☐ | ☐ | ☐ | ☐ | static MP2T file publish |
| GStreamer rtspsrc | ☐ | ☐ | ☐ | ☐ | ☐ | ☐ | `gst-launch-1.0 rtspsrc location=...` (we're client) |
| VLC | ☐ | ☐ | ☐ | ☐ | ☐ | ☐ | VLC > Stream > RTSP |
| Hikvision DS-2CD2xxx | ☐ | ☐ | ☐ | ☐ | ☐ | ☐ | common Digest MD5 |
| Axis P1xxx | ☐ | ☐ | ☐ | ☐ | ☐ | ☐ | Digest MD5/SHA-256 |
