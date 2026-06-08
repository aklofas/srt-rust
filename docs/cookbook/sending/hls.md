# Publish MPEG-TS as an HLS stream

> **⚠️ Experimental — not in the v0.1.0 published artifacts.** The HLS
> publisher is **Rust-only** and gated behind the `hls` Cargo feature
> (default-off). It is **not** enabled in the PyPI wheels or the JVM fat JAR.
> The built-in HTTP server has a pending security review (path traversal,
> a spec-violating `TARGETDURATION` floor, and unserved VOD playlists). Use
> it only for local / experimental builds with `--features hls`. See
> [`/docs/project/deferred-features.md`](/docs/project/deferred-features.md)
> for the full rationale and the trigger to promote it to a supported feature.

`tst-tcp`'s HLS publisher segments your stream to `.ts` files on disk and
serves them (plus a rolling `.m3u8`) over a built-in HTTP server. KLV
metadata stays inside the segments — STANAG 4609-aware players continue
to decode telemetry.

## Code

```rust
use std::time::Duration;
use tst_core::publisher::Publisher;
use tst_pipeline::MuxPublisher;
use tst_tcp::hls::{HlsMode, HlsPublisherBuilder};

let publisher = HlsPublisherBuilder::new()
    .bind("0.0.0.0:8080".parse()?)
    .output_dir("/var/cache/hls")
    .segment_duration(Duration::from_secs(4))
    .playlist_window(6)
    .mode(HlsMode::Live)
    .build()?;
println!("serving http://{}/playlist.m3u8", publisher.local_addr().unwrap());

let mut shell = MuxPublisher::with_config(publisher, mux_config)?;

// send_video with key_frame=true automatically cuts a new segment.
shell.send_video(&nal_bytes, pts, key_frame)?;
shell.send_klv(&klv_bytes, pts, 0)?;

// Optional Basic auth: .basic_auth("user", "secret") on the builder.
// Optional HTTPS:      .enable_tls("server.crt", "server.key") on the builder.

let publisher = shell.finish()?;
publisher.finish()?;  // writes final playlist + #EXT-X-ENDLIST (Event/Vod modes)
```

## Verify with ffmpeg / VLC / mpv

```bash
ffplay 'http://localhost:8080/playlist.m3u8'
vlc    'http://localhost:8080/playlist.m3u8'
mpv    'http://localhost:8080/playlist.m3u8'
```

## Modes

| Mode | Playlist behavior | ENDLIST written? | Disk eviction? |
|---|---|---|---|
| `HlsMode::Live` | Rolling window (`playlist_window` newest segments visible) | Never | Yes — segments rolled out of window are deleted |
| `HlsMode::Event` | Monotone-growing | On `finish()` | No |
| `HlsMode::Vod` | Same as Event | On `finish()` | No |

## URL parameters (when constructed via `from_url`)

| Parameter | Default | Meaning |
|---|---|---|
| `output_dir` | `/tmp/hls` | Filesystem dir for `.ts` + `.m3u8` |
| `segment_duration` | `4` (seconds) | Duration cap before forced cut |
| `playlist_window` | `6` | Rolling window size (LIVE mode only) |
| `mode` | `live` | One of `live`/`event`/`vod` |
| `auth_user`/`auth_pass` | none | Basic auth credentials |
| `cert`/`key` | none | HTTPS cert + key paths (PEM) |

## See also

- [Send MPEG-TS over TCP](tcp.md) — same crate, raw TCP variant
- [Send MPEG-TS over SRT](11-sender-from-url.md) — for reliable point-to-point
