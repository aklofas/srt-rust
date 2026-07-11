# HLS Publisher Guide


> **Who this is for:** You want to serve a live (or completed) MPEG-TS
> stream as HLS — `.ts` segments plus a rolling `.m3u8` playlist — to a
> browser-based player or a downstream CDN, with MISB KLV telemetry riding
> along inside the segments.

> **You will learn:**
> - What the HLS publisher is (a segmenter first, an HTTP server second)
> - The minimal Rust and Python publish loops
> - LIVE / EVENT / VOD modes and `finish_serving` semantics
> - How to serve segments in production (reverse proxy / CDN / built-in server)
> - How KLV rides the segments and how a web player reads it
> - The segment-boundary guarantees (PAT → PMT → IDR) and latency tuning

## What it is

The HLS publisher lives in its own crate, `tst-hls`. It is **segmenter
first**: it takes pre-muxed MPEG-TS bytes (or elementary streams, through
the `MuxPublisher` shell), cuts them into `.ts` segments on decodable
boundaries, and maintains an RFC 8216 media playlist (`playlist.m3u8`) on
disk. A built-in HTTP server that serves those files is an optional
convenience for development and edge deployments — production deployments
usually front the output directory with a real web server or CDN instead.

The publisher plugs into the same `tst_core::publisher::Publisher` trait as
any other segmented sink, so the `MuxPublisher` pipeline shell drives it the
same way `MuxSender` drives a transport: push video / KLV / audio, and the
shell muxes + segments + writes the playlist for you.

`tst-hls` is supported: it ships default-on in the Python wheels
(`tstrans.hls`) and behind the opt-in `hls` Cargo feature in the C binding
(`TST_HAS_HLS`). The JVM binding does not yet expose it — see
[deferred-features.md](/docs/project/deferred-features.md).

## Quickstart (Rust)

Drive the publisher through `MuxPublisher`, pushing elementary streams:

```rust
use std::time::Duration;
use tst_core::mpegts::common::Pts90khz;
use tst_core::publisher::Publisher;
use tst_hls::{HlsMode, HlsPublisherBuilder};
use tst_pipeline::MuxPublisher;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let publisher = HlsPublisherBuilder::new()
        // Default bind is 127.0.0.1:8080 (loopback). Bind all interfaces
        // (0.0.0.0) only behind a reverse proxy or with auth + TLS.
        .output_dir(std::env::temp_dir().join("hls-demo"))
        .segment_duration(Duration::from_secs(4))
        .playlist_window(6)
        .mode(HlsMode::Live)
        .build()?;
    if let Some(addr) = publisher.local_addr() {
        println!("serving http://{addr}/playlist.m3u8");
    }

    let mut shell = MuxPublisher::with_config(publisher, mux_config)?;

    // send_video with key_frame=true cuts a new segment at the keyframe.
    shell.send_video(&nal_bytes, Pts90khz::new(pts), /* key_frame */ true)?;
    shell.send_klv(&klv_bytes, Pts90khz::new(pts), 0)?;

    // LIVE mode: drop the shell to stop. To keep a completed EVENT/VOD
    // playlist fetchable, use finish_serving() below instead.
    let publisher = shell.finish()?;
    publisher.finish()?; // writes the terminal playlist + #EXT-X-ENDLIST
    Ok(())
}
```

Optional builder knobs: `.basic_auth("user", "secret")`, `.enable_tls(cert,
key)` (requires the `tls` feature), `.max_segment_duration(dur)` (see
[Segment guarantees](#segment-guarantees)).

## Quickstart (Python)

The Python surface mirrors the Rust one. Wheels ship it default-on:

```python
from tstrans.hls import HlsPublisher, MuxPublisher
from tstrans.mpegts import (
    KlvStreamType, MuxerProgramConfigBuilder, Pts90khz, VideoCodec,
)

publisher = (
    HlsPublisher.builder()
    .bind("127.0.0.1:0")               # 0 = ephemeral; read back via local_addr()
    .output_dir("/tmp/hls-demo")
    .segment_duration_ms(4000)
    .build()
)
print("serving http://%s/playlist.m3u8" % publisher.local_addr())

program = (
    MuxerProgramConfigBuilder(program_number=1, pmt_pid=0x1000)
    .add_video(0x1011, VideoCodec.H264)
    # PrivateData KLV (stream_type 0x06 + KLVA) is the web-friendly carriage.
    .add_klv(0x1031, KlvStreamType.PRIVATE_DATA, carries_pts=True)
    .build()
)

# with_config_hls CONSUMES the publisher.
mp = MuxPublisher.with_config_hls(publisher, program)
mp.send_video(nal, pts=Pts90khz.from_raw(pts), key_frame=True)  # auto-cuts on key_frame
mp.send_klv(klv, pts=Pts90khz.from_raw(pts))

pub = mp.finish_into_publisher()   # recover the publisher to finish it cleanly
pub.finish()
```

## Modes and `finish_serving`

| Mode | Playlist behavior | ENDLIST written? | Disk eviction? |
|---|---|---|---|
| `HlsMode::Live` | Rolling window (`playlist_window` newest segments visible) | Never during the run | Yes — segments rolled out of the window are deleted |
| `HlsMode::Event` | Monotone-growing (no eviction) | On `finish` | No |
| `HlsMode::Vod` | Written all at once when the stream ends | On `finish` | No |

`Publisher::finish` writes the terminal playlist and tears the server down.
For EVENT and VOD that is usually **not** what you want — the point of a VOD
is that it stays watchable after the stream ends. Use `finish_serving`
instead: it finalizes the playlist but keeps the built-in HTTP server up,
returning an `HlsServerHandle`.

```rust
// Finish the stream but keep serving the completed VOD/EVENT playlist
// and its segments until the handle is dropped or .shutdown() is called.
let handle = publisher.finish_serving()?;
println!("VOD available at http://{}/playlist.m3u8", handle.local_addr());
// ... hold the handle for as long as clients should be able to fetch ...
handle.shutdown();
```

In Python, `HlsPublisher.finish_serving()` returns the same
`HlsServerHandle` (with `local_addr()` / `shutdown()`).

## Serving in production

The built-in HTTP server is a **development and edge convenience**. It binds
loopback (`127.0.0.1:8080`) by default; binding all interfaces (`0.0.0.0`)
is an explicit choice you have to make, and even then the recommended
production shapes are:

- **Front the `output_dir` with a static web server or CDN.** The publisher
  only writes `segment_*.ts` and `playlist.m3u8` to a directory; point
  nginx, Caddy, a media server, or a CDN origin at that directory and let it
  serve the files. This is the highest-throughput, best-cached option and
  needs no traffic through this process at all.
- **Reverse-proxy the built-in server.** If you want the built-in server to
  do the serving (e.g. at the edge), keep it on loopback and put nginx / a
  media server in front for TLS termination, auth, and access control.
- Enable `basic_auth` and/or `enable_tls` on the builder if the built-in
  server must face untrusted clients directly, but a reverse proxy is the
  more flexible option.

Concurrent publishers must each use a **distinct `output_dir`** (and a
distinct bind port if each runs its own server) — two publishers writing the
same directory will clobber each other's segment numbering and playlist.

## KLV ride-along

KLV telemetry travels **inside the segments** as a normal MPEG-TS
elementary stream — there is no separate metadata channel. Choose the
carriage mode when you configure the program's KLV stream:

- **`KlvStreamType::PrivateData`** — PMT `stream_type = 0x06` plus a `KLVA`
  registration descriptor. This is the **web-friendly** path: hls.js ≥ 1.7
  with `enableEmsgKLVMetadata: true` surfaces these as `misbklv` samples,
  and older hls.js can slice the KLV out of the sample bytes by anchoring on
  the 16-byte SMPTE UL. Pass raw KLV LS bytes to `send_klv`.
- **`KlvStreamType::SynchronousMetadata`** — PMT `stream_type = 0x15`, the
  STANAG-strict carriage. The muxer prepends the 5-byte
  `Metadata_AU_cell` header (ITU-T H.222.0 §2.12.4.2) automatically — you
  still pass raw KLV LS bytes, not a pre-wrapped cell. Use this for STANAG
  4609 / MISB toolchains that expect 0x15.

For a browser client, use `PrivateData`. For a downstream STANAG toolchain,
use `SynchronousMetadata`. The recipe
[KLV-over-HLS to a browser](/docs/cookbook/sending/hls-klv-to-web.md) shows
the full producer + JavaScript client contract for both hls.js paths.

## Segment guarantees

- **Segments open on a decodable boundary.** Each segment begins with the
  PAT, the PMT, and an IDR (keyframe) access unit — in that order — so a
  player joining the stream can decode the first segment it fetches without
  waiting for a later keyframe.
- **Per-GOP duration.** When you push through `MuxPublisher`, a
  `send_video(..., key_frame=true)` cuts a segment at that keyframe, so
  segment length follows your GOP structure. `segment_duration` is the
  *target* — segments cut on the keyframe at or after that duration.
- **`max_segment_duration` force-cut.** In the keyframe-driven flow, if a
  keyframe is overdue (a stalled encoder or a very long GOP), the publisher
  force-cuts once the open segment exceeds `max_segment_duration` so
  segments never grow unbounded. It defaults to `2 × segment_duration` and
  must be `≥ segment_duration`. Each force-cut increments the
  `forced_cuts` stats counter — a persistently non-zero `forced_cuts` means
  your GOP cadence and `segment_duration` are mismatched.
- **`#EXTINF` reflects media (PTS) duration** when driven through
  `MuxPublisher`; the raw `push_ts` relay path falls back to wall-clock.
- **`#EXT-X-TARGETDURATION`** is fixed at `ceil(segment_duration)` and never
  changes after the playlist opens (RFC 8216 requires it immutable).
- **LIVE window floor.** RFC 8216 §6.2.2 requires a live playlist to hold at
  least three target durations; `HlsPublisherBuilder::build()` rejects a
  `playlist_window` too small to meet
  `playlist_window × segment_duration ≥ 3 × ceil(segment_duration)`.

## Latency tuning

Glass-to-glass latency in classic HLS is dominated by segment length and how
many segments a player buffers before it starts:

- **`segment_duration` vs GOP.** A player typically needs a few segments in
  the playlist before it begins, so shorter segments lower startup latency.
  Because a segment cuts on a keyframe, `segment_duration` can only be as
  fine as your **GOP length** — a 2-second GOP cannot produce 1-second
  segments. To cut latency, shorten the encoder's GOP first, then
  `segment_duration` to match.
- **Player buffering.** With hls.js, `liveSyncDurationCount` controls how far
  from the live edge playback starts (in segment counts). Lowering it (e.g.
  to `2`–`3`) trims latency at the cost of less rebuffer headroom on a lossy
  link.
- For sub-two-second latency you want LL-HLS (partial segments), which is
  deferred and couples to a CMAF/fMP4 packaging path — see
  [deferred-features.md](/docs/project/deferred-features.md).

## See also

- [Recipe: Publish MPEG-TS as an HLS stream](/docs/cookbook/sending/hls.md) — the copy-paste sender loop.
- [Recipe: KLV-over-HLS to a browser (hls.js)](/docs/cookbook/sending/hls-klv-to-web.md) — the full web-client contract, both hls.js paths.
- [guides/mpegts-mux.md](/docs/guides/mpegts-mux.md) — the muxer that feeds the segmenter.
- [guides/klv.md](/docs/guides/klv.md) — encoding the KLV that rides the segments.
- [guides/pipeline.md](/docs/guides/pipeline.md) — the `MuxPublisher` shell and its `MuxSender` siblings.
