# Python bindings (`tstrans`)

> **Who this is for:** You write Python and want to inspect, build,
> stream, or receive MPEG-TS + KLV — offline files *and* live transports
> (UDP, TCP, RTP/RTSP, SRT, RIST).

> **You will learn:**
> - How to install `tstrans` (with or without the pandas extra)
> - How to read a `.ts` file and inspect typed `DemuxEvent` items in ~5 lines
> - How to build a `.ts` file by pushing video + KLV through the `Muxer`
> - How to encode and decode all 4 MISB KLV sets (ST 0601 / 0102 / 0605 / 0903)
> - How to send and receive live MPEG-TS over UDP, TCP, RTP/RTSP, SRT, and RIST
> - How to pair video with synchronized KLV via `tstrans.pipeline.Pairer`
> - How to drive bulk KLV → pandas DataFrame ETL with the optional `[pandas]` extra
> - The Python-specific gotchas: GIL release, dataclass strictness, optional extras
> - How this binding differs from the Rust core

## Install

> **Not yet on PyPI.** `tstrans` has not been published to PyPI yet — the
> first PyPI release will be **v0.2.0**. Until then, install from source
> (build the wheel with `maturin`, or `maturin develop` in `bindings/python/`).
> The `pip install tstrans` commands below work once v0.2.0 is published.

```bash
pip install tstrans
```

Optional extras:

```bash
pip install 'tstrans[pandas]'   # pandas DataFrame adapters + NumPy snapshot views
```

**Minimum Python is 3.10** (bumped from 3.9 mid-Phase-2 to enable PEP 604
union syntax and `match` statements without compat hacks).

The compiled extension is imported as `tstrans._native`. Public API lives
on `tstrans` and its topic submodules — `tstrans.io`, `tstrans.mpegts`,
`tstrans.klv`, `tstrans.codec`, the live-transport modules `tstrans.srt`,
`tstrans.rtp`, `tstrans.udp`, `tstrans.tcp`, `tstrans.rist`, the
`tstrans.pipeline` pairer, and the optional `tstrans.pandas`. Don't reach
into `tstrans._native` directly; it may reorganize between versions.

Wheels (publishing with v0.2.0) ship the UDP, TCP, RTP/RTSP, SRT, and RIST transports
on by default — with two caveats: **RIST is excluded from the Windows
wheel** (the Linux and macOS wheels include it), and the **experimental
HLS publisher (`tstrans.hls`) is not in any published wheel** — it is
available only in a source build compiled with `--features hls` (see
[HLS publisher](#hls-publisher-tstranshls) below).

> **Status:** `tstrans` ships the full surface — offline file inspection
> + construction (`Demuxer` / `Muxer` / `MuxerFileSink`), typed KLV
> decode + encode for ST 0601 / ST 0102 / ST 0605 / ST 0903 (with
> `VTargetPack`), codec parsers for H.264 / H.265 / H.266 / AV1 / AAC /
> MPEG-2 audio, optional pandas DataFrame adapters + NumPy snapshot views
> via `pip install tstrans[pandas]`, the live transports
> `tstrans.{srt,rtp,udp,tcp,rist}` (with RTSP client + server and SRT
> auto-reconnect), and `tstrans.pipeline.Pairer`. ~1149 pytest tests.

## Hello world

Read a `.ts` file and print the type of each event in five lines:

```python
import tstrans

for event in tstrans.io.parse_file("capture.ts"):
    print(type(event).__name__)
```

## First send

Build a single-program H.264 `.ts` file by pushing one access unit through
the `Muxer`:

```python
from tstrans.mpegts import (
    KlvStreamType,
    Muxer,
    MuxerConfigBuilder,
    MuxerProgramConfigBuilder,
    Pts90khz,
    VideoCodec,
)

prog = (
    MuxerProgramConfigBuilder(program_number=1, pmt_pid=0x100)
    .add_video(0x101, VideoCodec.H264)
    .add_klv(0x102, KlvStreamType.SYNCHRONOUS_METADATA, carries_pts=True)
    .build()
)
cfg = MuxerConfigBuilder().add_program(prog).build()
m = Muxer(cfg)

with m.write_file("out.ts") as proxy:
    proxy.push_video(nal_bytes, pts=Pts90khz.from_raw(900_000))
    proxy.push_klv(klv_bytes, pts=Pts90khz.from_raw(900_000))
```

`MuxerFileSink` (the object returned by `write_file`) is a context
manager — `__exit__` flushes and finalizes the file; no explicit
`close()` ceremony is needed. Note the pushes go through `proxy` (the
object the `with` statement yields), not through `m` — only proxy
pushes drain to the file as they go.

## First receive

Demux a file and dispatch on typed events with a `match` statement:

```python
from tstrans.io import parse_file
from tstrans.mpegts import DemuxEvent

for event in parse_file("capture.ts"):
    match event:
        case DemuxEvent.ProgramMap(programs=pms):
            print(f"PSI: {len(pms)} programs")
        case DemuxEvent.Video(pts=p, codec=c, raw=b) as ev:
            # raw-first: `raw` is the exact encoded access unit. Splitting
            # it into typed NAL/OBU units is opt-in via `ev.parse()`.
            print(f"Video {c.name} pts={p.ms}ms len={len(b)} units={len(ev.parse())}")
        case DemuxEvent.Klv(pts=p, payload=b):
            print(f"KLV pts={p.ms}ms len={len(b)} (use tstrans.klv to decode)")
```

For a quick summary without iterating every event, use `probe`:

```python
from tstrans.io import probe

r = probe("capture.ts")
print(r.video_codecs, r.audio_codecs, r.has_klv)
```

To pull typed KLV records directly:

```python
from tstrans.io import extract_klv
from tstrans.klv import UasDatalinkLs, parse_klv_universal

# Iterate typed KLV records from a .ts file
for pts, record in extract_klv("capture.ts", parsed=True, with_pts=True):
    if isinstance(record, UasDatalinkLs):
        pos = record.sensor_position()
        if pos is not None:
            print(
                f"{pts.ms}ms platform={record.platform_designation} "
                f"@ {pos.lat_deg:.5f},{pos.lon_deg:.5f} alt={pos.alt_m:.1f}m"
            )

# Or dispatch a single record by UL
record = parse_klv_universal(raw_klv_bytes)
# record is UasDatalinkLs | SecurityLs | PrecisionTimeStampPack | VmtiLs | None
```

For the common ST-0601-only case there is a dedicated iterator that
also carries each record's file-order KLV index (it counts every KLV
event, so indices line up with a later re-mux pass over the same
file), and the typed sets support copy-update via `with_()` — they
are frozen dataclasses, so attribute assignment raises
`FrozenInstanceError`:

```python
from tstrans.io import iter_uas_datalink

for pts, klv_index, record in iter_uas_datalink("capture.ts"):
    corrected = record.with_(sensor_lat_deg=33.5)  # frozen → copy-update
```

KLV demux events decode in place too: `ev.parse()` on a
`DemuxEvent.Klv` dispatches by universal label — the KLV counterpart
of the raw-first `Video.parse()` / `Audio.parse()`.

All 4 MISB typed sets (ST 0601 UAS Datalink, ST 0102 Security,
ST 0605 Precision Time Stamp, ST 0903 VMTI) decode with the same
semantics as the Rust crate: lenient mode tolerates broken input and
accumulates per-field errors on `.field_errors`; strict mode raises
`tstrans.exceptions.KlvError`. Symmetric encoders (`encode_*_lenient`
/ `encode_*_strict`) round-trip parsed records back to wire bytes.
See the `tstrans.klv` module docstring for the full type listing.

## Transmux: edit metadata, copy everything else

`tstrans.io.transmux` bridges a demuxer and a muxer: iterate the source's
events and write back the ones to keep. Video/audio are copied
byte-for-byte via their raw encoded AUs; KLV can be substituted — pair
with `tstrans.klv.patch_uas_datalink` for byte-faithful tag edits. The
output muxer is built lazily from the first `ProgramMap`, so the
source's program topology (PIDs, codecs, program number) is reproduced.

```python
import tstrans.io as tio
from tstrans import klv
from tstrans.mpegts import DemuxEvent

with tio.transmux("in.ts", "out.ts", atomic=True) as tx:
    for ev in tx:
        if isinstance(ev, DemuxEvent.Klv):
            patched = klv.patch_uas_datalink(
                ev.payload, {"frame_center_lat_deg": 37.7749}
            )
            tx.write_klv(ev, patched)
        else:
            tx.write(ev)  # video/audio copied byte-for-byte
```

Strict by default: streams the muxer cannot represent (DVB
subtitling/teletext) raise `MuxError` naming the offenders.
Private/application data streams (unknown stream types) pass through
byte-faithfully: `MuxerConfig.from_program_map` reproduces their PMT
entry (raw stream_type byte + descriptor loop verbatim) and each
`DemuxEvent.UnknownSample` payload is re-emitted as-is via
`push_data_to`. Re-muxed data streams always carry PTS and the
demuxer substitutes 0 for a PTS-less source PES, so a source sample
with no PTS re-emerges with a literal PTS of 0.
Pass kinds in `drop=` (e.g. `drop=(StreamKindTag.UNKNOWN,)`) to
exclude streams instead; their events are then skipped by `write`. v1
supports single-program sources (a second program raises
`ValueError`).
`atomic=True` writes through a same-directory `*.partial` temp file and
`os.replace`s it into place only on clean exit, so no partial output can
appear at the destination.

## SRT transport (`tstrans.srt`)

`tstrans.srt` wraps `tst_pipeline`'s `Sender` / `Receiver` over an SRT
transport plus the low-level `tst_srt` `Builder` / `Socket` / `Listener`,
for streaming pre-muxed MPEG-TS bytes over SRT. Use the raw `Sender` /
`Receiver` when you already hold TS bytes (e.g. from a `Muxer` or a file);
use the `MuxSender` / `DemuxReceiver` convenience shells (next section) to
push elementary streams directly. SRT ships in published wheels
(default-on).

`SrtError` and `SrtErrorKind` live in `tstrans.exceptions`, **not** in
`tstrans.srt` (the same pattern as `tstrans.rtp`):

```python
from tstrans.exceptions import SrtError, SrtErrorKind
```

### Sender hello

Caller mode is the default when `?mode=` is omitted. URL query parameters
(`passphrase`, `latency`, `streamid`, `mss`, `payloadsize`, …) apply
through the SRT URL overlay:

```python
from tstrans.srt import Sender

# mode=caller is the default when ?mode= is omitted.
with Sender.from_url("srt://host:9000?mode=caller&passphrase=secret") as tx:
    tx.send_bytes(ts_bytes)   # pre-muxed TS bytes, any length
    tx.flush()                # emit any buffered partial bundle
```

`send_bytes` accepts a bytes-like payload of any length; the sender frames
it into 7-packet (1316-byte) SRT bundles internally. Call `flush()` after
the last push to emit a partial bundle. `socket_stats()` returns the
scheme-neutral 16-field `SocketStats`; `srt_stats()` returns the
libsrt-rich `SrtStats` (estimated bandwidth, symmetric send/recv-side loss
split).

### Receiver hello

`Receiver.from_url` does a one-shot bind + accept (listener mode). Each
`recv_bytes()` returns one 188-byte TS packet:

```python
from tstrans.srt import Receiver
from tstrans.exceptions import SrtError, SrtErrorKind

with Receiver.from_url("srt://:9000?mode=listener") as rx:
    try:
        while True:
            pkt = rx.recv_bytes()   # one 188-byte TS packet per call
            ...                     # process pkt
    except SrtError as e:
        # CLOSED / BROKEN both signal end of stream.
        if e.kind not in (SrtErrorKind.CLOSED, SrtErrorKind.BROKEN):
            raise
```

### Builder → Socket low-level path

When you need the listener's bound port (e.g. binding to an ephemeral
`:0`), drive the `Builder` → `Listener` → `Socket` path. A `Socket` is
consumed by exactly one of `into_sender()` / `into_receiver()` /
`into_mux_sender(program_config)` / `into_demux_receiver(demux_config=None)`:

```python
from tstrans.srt import Builder

# Caller side: connect, then turn the socket into a Sender.
with Builder("srt://host:9000").caller().latency_ms(200).connect() as sock:
    with sock.into_sender() as tx:
        tx.send_bytes(ts_bytes)
        tx.flush()

# Listener side: bind an ephemeral port, read it back, accept one peer.
with Builder("srt://127.0.0.1:0?mode=listener").listener().listen() as listener:
    host, port = listener.local_addr()
    print("listening on", port)
    sock = listener.accept()          # blocks for the first peer
    with sock.into_receiver() as rx:
        pkt = rx.recv_bytes()
```

The `Builder` mode setters (`.caller()` / `.listener()`) set only the
Python-side mode; `.listen()` still requires `?mode=listener` in the URL.
URL-provided values win over kwargs / setters. A `Listener` is iterable —
iterating yields accepted `Socket`s until `cancel_handle().cancel()` stops
it.

### Cancellation

`Sender` / `Receiver` / `Listener` (and the `DemuxReceiver` shell) expose
`cancel_handle()`; calling `.cancel()` from another thread wakes a thread
parked in `send_bytes` / `recv_bytes` / `accept` within ~3–10 ms,
surfacing `SrtError(BROKEN)` or `SrtError(CLOSED)`:

```python
tx = Sender.from_url("srt://host:9000?mode=caller")
cancel = tx.cancel_handle()
# On another thread:
cancel.cancel()   # wakes tx.send_bytes() → SrtError(BROKEN | CLOSED)
```

`is_cancelled()` is per-clone, but `cancel()` on any clone wakes the shared
socket.

### SRT convenience (`MuxSender` / `DemuxReceiver`)

`MuxSender` bundles a `Muxer` + an SRT `Sender`: push encoded elementary
streams and it muxes + sends in one call. `DemuxReceiver` bundles a
`Receiver` + a `Demuxer`: iterate to get the same
`tstrans.mpegts.DemuxEvent` subclass instances the offline `Demuxer`
produces. Both release the GIL around the muxer / transport work, and both
are context managers.

```python
from tstrans.srt import MuxSender
from tstrans.mpegts import MuxerProgramConfigBuilder, Pts90khz, VideoCodec

program = (
    MuxerProgramConfigBuilder(program_number=1, pmt_pid=0x1000)
    .add_video(0x1011, VideoCodec.H264)
    .build()
)

with MuxSender.from_url(
    "srt://host:9000?mode=caller&latency=120", program
) as s:
    pts = 0
    for nal in access_units:
        s.push_video(nal, pts=Pts90khz.from_raw(pts), key_frame=True)
        pts += 3000   # 90 kHz ticks per frame
```

`pts` is keyword-only on every push method. The push surface mirrors the
offline muxer: `push_video` / `push_klv` / `push_audio` / `push_subtitle`
/ `push_data` (raw private-data bytes, passed through verbatim), plus the
handle-targeted `push_*_to` variants for multi-stream programs and the
first-of-kind handle accessors (`video_handle()`, `klv_handle()`, …,
`data_handle()`). `stats()` returns a `(SocketStats, MuxerStats)` tuple.
There is no `flush()` — bytes flush per-push and again on `close()`.

Receiving:

```python
from tstrans.srt import DemuxReceiver
from tstrans.mpegts import DemuxEvent

with DemuxReceiver.from_url("srt://:9000?mode=listener") as rx:
    for event in rx:
        match event:
            case DemuxEvent.Video(pts=p, raw=b):
                ...   # one encoded access unit
            case DemuxEvent.Klv(payload=b):
                ...
```

`add_byte_sink(callback)` fans out every 188-byte TS packet (as `bytes`)
to a callback BEFORE demuxing — register it before iterating. If the
callback raises, the error re-raises from the next iteration step and
iteration stops. Keep the sink cheap (it runs on the recv-loop thread) and
never re-enter the receiver from it.

### SRT managed reconnect

The four `Managed*` shells add automatic reconnect: on a Broken/Closed
transport they re-dial (caller) or re-bind + re-accept (listener) under a
`ReconnectPolicy` and resume, replaying buffered gap data. `ManagedSender`
/ `ManagedReceiver` are the raw-bytes pair; `ManagedMuxSender` /
`ManagedDemuxReceiver` are the convenience pair.

```python
from tstrans.srt import (
    ManagedMuxSender,
    ReconnectPolicy,
    BackoffStrategy,
    OverflowPolicy,
)
from tstrans.mpegts import MuxerProgramConfigBuilder, Pts90khz, VideoCodec

policy = ReconnectPolicy(
    max_attempts=None,                  # None = retry forever; an int caps attempts
    backoff=BackoffStrategy.exponential(base_ms=100, max_ms=10_000),
    gap_buffer_capacity=256,
    overflow_policy=OverflowPolicy.DROP_OLDEST,
)

program = (
    MuxerProgramConfigBuilder(program_number=1, pmt_pid=0x1000)
    .add_video(0x1011, VideoCodec.H264)
    .build()
)

with ManagedMuxSender.from_url(
    "srt://host:9000?mode=caller&latency=120", program, policy=policy
) as s:
    for i, nal in enumerate(access_units):
        s.push_video(nal, pts=Pts90khz.from_raw(i * 3000), key_frame=True)
```

Passing `policy=None` (or omitting it) uses the defaults
(`max_attempts=10`, `BackoffStrategy.exponential(base_ms=100,
max_ms=10_000)`, `gap_buffer_capacity=256`, `OverflowPolicy.DROP_OLDEST`).
`BackoffStrategy.constant(ms)` is the fixed-wait alternative;
`OverflowPolicy` is `DROP_OLDEST` or `REJECT`; `gap_buffer_capacity == 0`
raises `ValueError`.

On the receive side, each reconnect emits exactly one
`DemuxEvent.ReconnectDiscontinuity` before the post-reconnect events —
drop your per-stream caches on receipt and rebuild from the next
`ProgramMap`:

```python
from tstrans.srt import ManagedDemuxReceiver
from tstrans.mpegts import DemuxEvent

with ManagedDemuxReceiver.from_url(
    "srt://host:9000?mode=caller&latency=120", policy=policy
) as rx:
    for event in rx:
        if isinstance(event, DemuxEvent.ReconnectDiscontinuity):
            caches.clear()     # transport rebuilt — re-derive from next ProgramMap
        elif isinstance(event, DemuxEvent.Video):
            ...
```

Unlike the plain `DemuxReceiver` (listener only), `ManagedDemuxReceiver`
accepts `mode=caller` too — it re-dials in caller mode and re-binds +
re-accepts in listener mode.

**Stats drift on the managed shells** (mirrors the JVM binding):
`ManagedSender.srt_stats()` and `ManagedReceiver.srt_stats()` raise
`SrtError(IO)` today — the managed transport exposes no SRT-rich shape, so
use `socket_stats()` (the 16-field `SocketStats`) instead.
`ManagedDemuxReceiver.srt_stats()` does NOT throw — it returns a
`SocketStats` (not `SrtStats`). `ManagedReceiver.reconnect_attempts()` is a
success count (excludes the initial accept); `ManagedMuxSender` and
`ManagedDemuxReceiver` `reconnect_attempts()` count every reconnect-factory
invocation.

## RTP transport (`tstrans.rtp`)

`tstrans.rtp` wraps `tst_rtp` directly for MPEG-TS-over-RTP/UDP
(RFC 2250): each `send` produces one RTP datagram (12-byte header + TS
payload); each `recv` returns one datagram's TS payload with the header
stripped. Unlike SRT, the raw `Sender` / `Receiver` are constructed with
`__init__`, not `from_url`. RTP ships in published wheels (default-on).
`RtpError` / `RtpErrorKind` and `RtspError` / `RtspErrorKind` live in
`tstrans.exceptions`.

### Sender / Receiver hello

```python
from tstrans.rtp import Sender, Receiver

# pkt_size caps the datagram (RTP header + TS payload); ssrc pins the SSRC.
with Sender("rtp://239.0.0.1:5004", pkt_size=1316) as tx:
    tx.send(ts_bytes)        # one UDP datagram per call

# Multicast bind URLs auto-join the group.
with Receiver("rtp://239.0.0.1:5004") as rx:
    payload = rx.recv()      # one datagram's TS payload (RTP header stripped)
```

`send` accepts a TS payload up to `pkt_size − 12`; oversize raises
`RtpError(MALFORMED_PACKET)`. `recv()` blocks until a datagram arrives or a
cancel fires. RTP/UDP is connectionless — a remote sender closing does NOT
end a `recv()` loop; stop on a sentinel or via `cancel_handle().cancel()`,
which wakes a parked `send` / `recv` with `RtpError(CANCELLED)` within
~100 ms. The RTP `CancelHandle` has only `cancel()` — no `is_cancelled()`
(differs from SRT). A literal `rtp://host:0` receiver is unusable: RTCP
auto-binds `port + 1`, which for port 0 lands on port 1 — bind a concrete
port.

### RTP convenience (`MuxSender` / `DemuxReceiver`)

`MuxSender` takes the same elementary-stream push surface as the SRT
shell, constructed with `MuxSender(url, program_config, *,
pkt_size=1316)`; `DemuxReceiver` iterates `DemuxEvent`s and is constructed
with `DemuxReceiver(url, *, demux_config=None)`. Both expose **no
`cancel_handle()` and no `socket_stats()`** — only `stats()` (a
`(SocketStats, MuxerStats)` tuple) and `close()`:

```python
from tstrans.rtp import MuxSender, DemuxReceiver
from tstrans.mpegts import (
    DemuxEvent,
    MuxerProgramConfigBuilder,
    Pts90khz,
    VideoCodec,
)

program = (
    MuxerProgramConfigBuilder(program_number=1, pmt_pid=0x1000)
    .add_video(0x1011, VideoCodec.H264)
    .build()
)
with MuxSender("rtp://127.0.0.1:5004", program) as s:
    s.push_video(nal, pts=Pts90khz.from_raw(0), key_frame=True)

with DemuxReceiver("rtp://0.0.0.0:5004") as rx:
    rx.add_byte_sink(lambda pkt: record(pkt))   # each raw 188-byte TS packet
    for event in rx:
        if isinstance(event, DemuxEvent.Video):
            ...
```

To stop a `DemuxReceiver` iteration parked on the next datagram, call
`close()` from another thread — it cancels the in-flight recv first, then
frees the receiver (safe cross-thread).

### RTSP client

`RtspClient.connect(config)` runs OPTIONS / DESCRIBE / SETUP / PLAY and
returns a live `RtspSession`; `RtspSession.into_demux_receiver()` hands you
the RTP data plane as a `DemuxReceiver`. Auth is `BasicAuth(user,
password, realm=None)` or `DigestAuth(user, password, algorithm=...,
realm=None)`:

```python
from tstrans.rtp import (
    RtspClient,
    RtspClientConfig,
    DigestAuth,
    TransportPref,
)

cfg = RtspClientConfig(
    "rtsp://cam.local:554/stream1",
    auth=DigestAuth("admin", "secret"),     # BasicAuth | DigestAuth | None
    transport_pref=TransportPref.AUTO,      # UDP-first, TCP fallback on 461
)

with RtspClient.connect(cfg) as session:
    with session.into_demux_receiver() as rx:
        for event in rx:
            ...     # DemuxEvent.Video / .Klv / ...
    # session.play() / session.pause() / session.teardown() drive RTSP state.
```

- **Passwords stay Rust-side.** `BasicAuth` / `DigestAuth` hold the
  password in Rust memory (never re-exposed); only `user`, `realm`, and
  `algorithm` are readable. `repr()` redacts the secret.
- **TLS is forward-compat only.** `rtsps://` surfaces `RtspError(TLS)`;
  `tls_root_certs_pem` is accepted for parity but is not read by `connect`.
- **Cancellation.** Obtain a `RtspCancelHandle` from
  `session.cancel_handle()` *before* a blocking control call, then flip
  `cancel()` from another thread to break it out.

### RTSP server

`RtspServer.start(config)` hosts a server; `add_unicast_mount(path,
program_config)` / `add_multicast_mount(path, group, port, *,
program_config=...)` register mounts, and the returned `MountHandle` takes
the same elementary-stream push family as `MuxSender` (the `push_*` /
`push_*_to` family plus per-kind handle accessors). The `MountHandle` is
`Arc`-shared, so multiple producer threads may push concurrently.

```python
from tstrans.rtp import RtspServer, RtspServerConfig
from tstrans.mpegts import MuxerProgramConfigBuilder, Pts90khz, VideoCodec

program = (
    MuxerProgramConfigBuilder(program_number=1, pmt_pid=0x1000)
    .add_video(0x1011, VideoCodec.H264)
    .build()
)

cfg = RtspServerConfig(bind_addr="0.0.0.0:8554")   # defaults: max_sessions=100, …
with RtspServer.start(cfg) as server:
    mount = server.add_unicast_mount("/live", program)
    mount.push_video(nal, pts=Pts90khz.from_raw(0), key_frame=True)
    print(server.local_addr())     # bound "ip:port" (useful with port 0)
# __exit__ sends a graceful Notice-5402 teardown to active sessions.
```

`RtspServerConfig` is a frozen dataclass (`bind_addr`, `auth`,
`max_sessions`, `session_timeout_secs`, `fanout_capacity`,
`graceful_shutdown_drain_ms`, `tls_cert_pem`, `tls_key_pem`). Its TLS
fields are forward-compat: setting `tls_cert_pem` / `tls_key_pem` raises
`RtspError(TLS)` at `start()` (and they must be set together or
`__post_init__` raises `ValueError`). For a credentialed server pass
`auth=BasicAuth("user", "pass", "realm")` — the realm is required
server-side. `server.cancel_handle()` returns an `RtspServerCancelHandle`
for an immediate hard teardown that bypasses the drain window.

## UDP / TCP / RIST transports

These three are the raw datagram / stream transports — lighter than SRT or
RTP (no muxer shells), used to move pre-muxed TS bytes. Each exposes a
fluent builder. All three ship in published wheels (RIST on Linux + macOS
only — see the caveat under [Install](#install)). Their error types
(`UdpError`, `TcpError`, `RistError` and the matching `*ErrorKind`
`IntEnum`s) live in `tstrans.exceptions` and are also re-exported from each
submodule.

### UDP (`tstrans.udp`)

```python
from tstrans.udp import Transport, RecvTransport

# Sender — fixed peer.
with Transport.builder().url("udp://239.0.0.1:5000").ttl(8).build() as tx:
    tx.send(ts_bytes)                       # one datagram; default cap 1316 bytes

# Receiver — bind, then recv (timeout_ms=None blocks).
with RecvTransport.builder().bind_url("udp://@239.0.0.1:5000").build() as rx:
    payload, _addr = rx.recv(timeout_ms=1000)   # raises UdpError(IO) on timeout
```

`recv()` returns `(payload, sender_addr)` — the address string is
currently always `""`. Use `udp://@group:port` (the `@` prefix) on
`bind_url` for a multicast join; `RecvTransport.local_addr_port()` reads
back an ephemeral port when you bind `:0`.

### TCP (`tstrans.tcp`)

`Transport` is full-duplex (send + recv on one handle); a `Listener`
accepts inbound connections:

```python
from tstrans.tcp import Transport, Listener

# Client.
with Transport.builder().url("tcp://host:5001").nodelay(True).build() as conn:
    conn.send(ts_bytes)
    buf = bytearray(64 * 1024)
    n = conn.recv(buf)                       # bytes read into buf

# Server.
with Listener.builder().bind("0.0.0.0:5001").build() as listener:
    conn = listener.accept_blocking()        # -> Transport
    with conn:
        n = conn.recv(buf := bytearray(64 * 1024))
```

`tcps://` URLs raise `TcpError(TLS_DISABLED)` at `build()` — TLS is not
compiled into the published wheels. `Listener.builder().bind("host:0")`
plus `local_port()` gives you an ephemeral port.

### RIST (`tstrans.rist`)

```python
from tstrans.rist import Transport, RecvTransport, EncryptionKey

# Sender.
with Transport.builder().url("rist://host:5004").build() as tx:
    tx.send(ts_bytes)

# Receiver — the @ prefix is REQUIRED on bind_url.
with RecvTransport.builder().bind_url("rist://@0.0.0.0:5004").build() as rx:
    payload = rx.recv(timeout_ms=1000)       # raises RistError(RECV_TIMEOUT)

# Encryption forces RistProfile.MAIN.
enc = EncryptionKey.aes256("pre-shared-secret")
tx = Transport.builder().url("rist://host:5004").encryption(enc).build()
```

The `@` prefix on `bind_url` is mandatory (per librist / ffmpeg
convention); omit it on the sender's `url`. Mixing them up raises
`RistError(INVALID_CONFIG)`. `EncryptionKey.aes128` / `aes192` / `aes256`
each force `RistProfile.MAIN` (the profile that carries encryption); the
default is `RistProfile.SIMPLE`. Both transports return a `RistStats`
snapshot from `stats()`.

## HLS publisher (`tstrans.hls`)

> **Experimental — not in published wheels.** `tstrans.hls` is available
> only in a source build compiled with `--features hls`. From a PyPI
> wheel, `import tstrans.hls` raises `ImportError`. The surface and its
> on-disk / HTTP behavior may change.

`HlsPublisher` segments pre-muxed MPEG-TS to disk and serves an HTTP
playlist; `MuxPublisher` is the pipeline shell that owns a muxer + an
`HlsPublisher` so you can push elementary streams instead of TS bytes:

```python
from tstrans.hls import HlsPublisher, MuxPublisher
from tstrans.mpegts import MuxerProgramConfigBuilder, Pts90khz, VideoCodec

publisher = (
    HlsPublisher.builder()
    .bind("127.0.0.1:0")               # 0 = ephemeral; read back via local_addr()
    .output_dir("/tmp/hls")
    .segment_duration_ms(2000)
    .build()
)

program = (
    MuxerProgramConfigBuilder(program_number=1, pmt_pid=0x1000)
    .add_video(0x1011, VideoCodec.H264)
    .build()
)

# with_config_hls CONSUMES the publisher.
mp = MuxPublisher.with_config_hls(publisher, program)
mp.send_video(nal, pts=Pts90khz.from_raw(0), key_frame=True)  # auto-cuts on key_frame
pub = mp.finish_into_publisher()   # recover the publisher to finish() it cleanly
pub.finish()
```

`push_ts` (on the publisher) requires a 188-multiple and raises
`HlsError(UNALIGNED_PUSH_TS)` otherwise; once consumed (by `finish()` or
`with_config_hls`) further calls raise `HlsError(FINISHED)`.
`HlsMode.LIVE` / `EVENT` / `VOD` selects playlist behavior.

The full `Publisher` ABC surface exposed through `tstrans.hls`:

| Method | Type | Notes |
|---|---|---|
| `push_ts(data: bytes) -> None` | `HlsPublisher` | 188-multiple buffer; raises `HlsError(UNALIGNED_PUSH_TS)` otherwise |
| `cut_segment() -> None` | `HlsPublisher` | Wall-clock segment cut hint; call on keyframe boundaries |
| `cut_segment_with_duration(media_duration_us: int) -> None` | `HlsPublisher` | Media-presentation-time-derived cut; `media_duration_us` is the PTS span of the completed segment in µs (same unit as `PublisherStats.*_us`); used by `MuxPublisher` internally |
| `finish() -> None` | `HlsPublisher` | Flush pending segment + write `#EXT-X-ENDLIST`; terminal |
| `stats() -> PublisherStats` | `HlsPublisher` / `MuxPublisher` | Cross-publisher stats snapshot |

## Pipeline pairing (`tstrans.pipeline.Pairer`)

MPEG-TS programs that carry synchronized KLV (e.g. MISB ST 0601 UAS
Datalink) multiplex video on one PID and KLV on another, both timestamped
against the same 90 kHz clock but arriving in separate PES packets.
`Pairer` — wrapping the Rust core `PairingDemuxer` — correlates the two by
PTS without exposing demux events across the FFI boundary: you feed raw TS
bytes and get back `PairerOutput`s. `Pairer` is **not** a context manager
and is single-threaded (the consumer owns concurrency).

```python
from datetime import timedelta
from tstrans.pipeline import (
    Pairer,
    PairerConfig,
    PairerMode,
    PairerOutput,
    PairingDemuxerConfig,
)

video_pid, klv_pid = 0x101, 0x102

# Realtime nearest-PTS pairing, 100 ms tolerance. config=None → defaults
# (Realtime, 300 ms tolerance, 32/32 buffers, link_klv_to_video=True).
cfg = PairingDemuxerConfig(
    pairer=PairerConfig(
        mode=PairerMode.Realtime,
        tolerance=timedelta(milliseconds=100),
    ),
)

pairer = Pairer(video_pid, klv_pid, cfg)
outputs = pairer.feed(ts_bytes)
outputs += pairer.flush()              # drain end-of-stream (trailing UnpairedKlv; buffered video too)

for out in outputs:
    match out:
        case PairerOutput.Paired(video=v, klv=k):
            # v.codec, v.payload (list[NalUnit] | list[Obu]); k.payload (bytes)
            ...
        case PairerOutput.UnpairedVideo(video=v):
            ...
        case PairerOutput.UnpairedKlv(klv=k):
            ...
        case PairerOutput.PassThrough(event=ev):
            ...   # a tstrans.mpegts.DemuxEvent.* instance (ProgramMap, off-PID, …)

print(pairer.stats())   # {'paired': N, 'unpaired_video': N, 'unpaired_klv': N, 'pass_through': N}
```

The simplest form — `Pairer(video_pid, klv_pid)` — uses all defaults. To
tolerate arrival skew, switch to Buffered mode by passing
`mode=PairerMode.Buffered(max_lag=timedelta(milliseconds=200))` to
`PairerConfig`; `flush()` is most load-bearing in Buffered mode, where
buffered samples are held until the lag window closes. Call `flush()` at
end-of-stream in **either** mode, though: it drains any unused KLV history as
trailing `UnpairedKlv` (e.g. metadata that arrived after the last video access
unit), so skipping it can drop tail metadata. `feed` and `flush`
each return a `list[PairerOutput]`; `feed` raises
`tstrans.exceptions.DemuxError` on non-conformant input. `stats()` and
`demuxer_stats()` return dicts; `reset_stats()` zeroes the pairing
counters without touching demuxer stats.

## Language-specific gotchas

- **GIL released in `push_*` methods.** Long-running CPU work (large NAL
  parses, big KLV blobs) doesn't block other Python threads. The
  `add_subtitle()` and `push_subtitle*()` methods also release the GIL
  (added in plan #96 Wave C).
- **Subtitle config dataclasses reject `bool`-as-`int`.** PyO3 strictness
  means `True` is not silently coerced to integer `1` for fields that
  expect an integer codec selector. Same with `bytearray` vs `bytes`.
  (Came from plan #96 validation pass.)
- **`MuxerFileSink` is a context manager — push on the proxy it
  yields.** Use `with m.write_file("out.ts") as proxy: ...` and route
  every `push_*` through `proxy`. Only proxy pushes drain to the file;
  pushing on the original Muxer (`m.push_video(...)`) inside the block
  bypasses the per-push drain and raises `MuxError(BACKPRESSURE)` once
  `buffer_packets` (default 10 000) accumulate — a footgun that only
  fires in long push loops. The `__exit__` flushes + finalizes the
  file. No explicit `close()` ceremony is needed (and a double-close on
  the underlying handle would panic).
- **Video / Audio events are raw-first; parsing is opt-in.** A
  `DemuxEvent.Video` / `DemuxEvent.Audio` carries `.raw` (the exact encoded
  bytes). Call `ev.parse()` to get typed units: for H.264 / H.265 / H.266
  video it's `list[NalUnit]`; for AV1 it's `list[Obu]`; for AAC ADTS it's
  `list[AdtsFrame]`; for MPEG-2 Audio it's `list[Mpeg2AudioFrame]`. For
  subtitles + AAC-LATM + AC-3 there's no typed frame parser yet — use `.raw`
  directly (AC-3 carriage is still sync-validated on demux). The
  free functions `tstrans.codec.split_units(raw, codec)` and
  `tstrans.codec.parse_audio(raw, codec)` do the same split and additionally
  return the conformance-issue list.
- **abi3 build limitation.** `bytes`-like extraction uses a two-path
  approach (one for true `bytes`, one for `memoryview` / `bytearray`)
  because PyO3's abi3 doesn't expose a unified buffer protocol. The
  Python API is uniform — you can pass `bytes`, `bytearray`, or a
  `memoryview` and it works.
- **`tstrans._native` is private.** Use `tstrans.X` (or `tstrans.mpegts.X`
  / `tstrans.klv.X` / ...) — never `tstrans._native.X`. The `_native`
  submodule may reorganize between versions.

### Pandas + NumPy adapters

Optional pandas DataFrame adapters and NumPy snapshot views (one
Rust-to-Python `bytes` copy per access; see [Snapshot vs zero-copy](#snapshot-vs-zero-copy)
below) for the `tstrans` Python package. Requires the `[pandas]` extra:

```bash
pip install 'tstrans[pandas]'
```

Without the extra, `tstrans` works as documented in the core modules above. Calling any pandas adapter or any
NumPy `.payload_np` / `.raw_rbsp_np` / `.raw_np` accessor without the
extra raises:

```
ImportError: tstrans pandas adapters require: pip install 'tstrans[pandas]'
```

#### Quick start

```python
import tstrans.io
import tstrans.pandas

# Parse a .ts file into events
events = list(tstrans.io.parse_file("capture.ts"))

# Convert to DataFrame for analysis
df = tstrans.pandas.events_to_dataframe(events)
print(df.kind.value_counts())
#  Sample                  1234
#  Metadata                  56
#  ProgramMap                12
```

#### DataFrame adapters

##### KLV records — `klv_to_dataframe`

```python
from tstrans.io import extract_klv

records = list(extract_klv("capture.ts", parsed=True))
df = tstrans.pandas.klv_to_dataframe(records)
df.head()
```

`klv_to_dataframe` is polymorphic — it dispatches on the record type
and produces a per-set schema. Input must be homogeneous (one set type
per call); mixed input raises `TypeError`. Supported types: `UasDatalinkLs`
(ST 0601), `SecurityLs` (ST 0102), `PrecisionTimeStampPack` (ST 0605),
`VmtiLs` (ST 0903).

KLV DataFrames are indexed by `pd.DatetimeIndex` (with `tz="UTC"`,
named `pts`) derived from the per-record timestamp where present:
ST 0601 / ST 0903 use the `timestamp_us` field (microseconds since
the 1970 UTC epoch), ST 0605 uses its own precision timestamp. If a
record lacks a timestamp the row's index entry is `pd.NaT`; if NO
record in the batch has one the DataFrame falls back to
`pd.RangeIndex`.

**Column shape.** ST 0601 (UasDatalinkLs) flattens to its full set of
~50 scalar fields — fields like `frame_center_lat_deg`,
`frame_center_lon_deg`, `frame_center_elev_m`, `sensor_lat_deg`,
`sensor_lon_deg`, `sensor_alt_m`, `platform_heading_deg`,
`platform_pitch_deg`, `platform_roll_deg` are direct top-level columns
(no dotted composite namespacing). Enum-valued fields collapse to their
variant name string (e.g. `"FullyEncrypted"`). Per-field parse errors
(Phase 3 `KlvFieldError`) collapse to a single string `field_errors`
column using a `|` joiner with the per-error format
`tag<N>:<kind>:<message>` — the `|` (not `,`) joiner keeps the column
parseable even when an error `message` contains commas.

**ST 0903 (VmtiLs) supports two modes:**

- `mode="summary"` (default): one row per VMTI record, with a
  `num_targets` column counting `VTargetPack` entries. Indexed by
  `pd.DatetimeIndex` of record timestamps.
- `mode="targets"`: one row per `VTargetPack`, indexed by
  `pd.MultiIndex` with levels `[pts, target_id]`.

```python
# Aggregate targets across the full capture
targets = tstrans.pandas.klv_to_dataframe(vmti_records, mode="targets")
```

##### DemuxEvents — `events_to_dataframe`

```python
df = tstrans.pandas.events_to_dataframe(events)
```

Union schema across all event kinds. Video / Audio / Subtitle events
collapse to `kind="Sample"`; KLV events collapse to `kind="Metadata"`;
ProgramMap / NonConformant / Discontinuity / ReconnectDiscontinuity
keep their own labels. (`Pat` is folded into `ProgramMap` by the
demuxer; it never appears as a separate kind.)

| Column | Type | Description |
|---|---|---|
| kind | str | `Sample` / `Metadata` / `ProgramMap` / `NonConformant` / `Discontinuity` / `ReconnectDiscontinuity` |
| pts_raw | u64 | `Pts90khz.raw` ticks |
| pts_ms | float | `Pts90khz.ms` (PTS in milliseconds) |
| dts_ms | float | DTS in ms (Sample events that carry it; otherwise NaN) |
| pid | u16 | Source PID (NaN for global events) |
| stream_type | str | `StreamKind` variant name (`Video` / `Audio` / `Klv` / `Subtitle`) |
| codec | str | Codec tag (`H264` / `H265` / `H266` / `Av1` / `Aac` / `Mpeg2Audio` / `WebVtt` / ...) |
| payload_len | int | byte length of the event payload — `len(raw)` for video / audio rows, `len(payload)` for KLV / subtitle rows |
| nal_count | int | Video-only — the per-AU unit count, obtained by running the opt-in `event.parse()` on each `_VideoEvent` row (NAL units, or OBUs for AV1). NaN on audio rows and on non-Sample rows |
| random_access | bool | TS adaptation-field RAI bit (video samples) |
| has_codec_parse_error | bool | Vestigial column, always `None` under the raw-first surface — the eager `codec_parse_error` field was dropped; conformance issues now surface via `event.parse(strict=True)` / `tstrans.codec.split_units`. Kept for schema stability. |
| issue | str | `NonConformant` event's issue text |
| issue_kind | str | `NonConformant` event's `.kind` enum variant name |

Payloads themselves stay on the original event objects — they're not
materialised in the DataFrame.

##### NAL / OBU lists — `nals_to_dataframe` / `obus_to_dataframe`

```python
# Extract NALs from a single video Sample (opt-in split via `.parse()`)
sample = next(e for e in events if type(e).__name__ == "_VideoEvent")
df = tstrans.pandas.nals_to_dataframe(sample.parse(), pts=sample.pts.ms)
df.nal_type_name.value_counts()
```

NAL type names are decoded via H.264 §Table 7-1 / H.265 §Table 7-1 /
H.266 V4 §Table 5 lookup keyed on `nal.kind`. Unknown types fall back
to `unknown_{n}`.

Columns: `kind`, `nal_type`, `nal_type_name`, `ref_idc` (H.264 only;
NaN elsewhere), `layer_id` (H.265/H.266 only; NaN on H.264),
`temporal_id_plus1`, `payload_len`, and `pts_ms` if the optional `pts`
argument was supplied.

```python
# AV1 sample (`.parse()` returns the OBU list)
df = tstrans.pandas.obus_to_dataframe(sample.parse(), pts=sample.pts.ms)
```

OBU schema: `obu_type`, `obu_type_name`, `temporal_id`, `spatial_id`
(both from the optional OBU extension; NaN when absent), `payload_len`,
and `pts_ms` if supplied.

##### Audio frames — `audio_frames_to_dataframe`

```python
from tstrans.codec import parse_aac_frames

frames = parse_aac_frames(buf)
df = tstrans.pandas.audio_frames_to_dataframe(frames)
df.plot(x="byte_offset", y="frame_length_bytes")
```

Polymorphic — detects `AdtsFrame` vs `Mpeg2AudioFrame` from the first
element. Mixed-type input raises `TypeError`. Enum-valued fields
collapse to their bare variant name (e.g. `"LC"`, not `"AacProfile.LC"`;
`"III"` for MPEG-2 Audio Layer III; `"JOINT_STEREO"` for the channel
mode). Struct-valued `AacChannelLayout` is kept as its `repr`.

`byte_offset` is the running cumulative offset of each parsed frame
inside the input buffer, computed by summing `frame_length_bytes`
from zero. For inputs produced by `parse_*_frames_with_resync`, this
does NOT account for skipped (garbage) bytes between recovered frames
— if you need absolute offsets across a resync boundary, pre-compute
them from the resync output itself.

#### NumPy snapshot views

Every byte-bearing class (NalUnit, Obu, AdtsFrame, Mpeg2AudioFrame, all
H.264/H.265/H.266 SPS/PPS/VPS/SliceHeaderLight, AV1 sequence/frame
headers) carries a `.payload_np` / `.raw_rbsp_np` / `.raw_np` accessor
that returns a `numpy.ndarray(dtype=uint8)` snapshot — each access
copies from Rust-owned storage into a fresh Python `bytes`, which
NumPy then views without further copy:

```python
import numpy as np
from tstrans.codec import parse_h264_sps

sps = parse_h264_sps(rbsp_bytes)
arr = sps.raw_rbsp_np   # snapshot np.ndarray(dtype=np.uint8)
```

Mapping:

- `.payload_np` — `NalUnit`, `Obu`, `AdtsFrame`, `Mpeg2AudioFrame`
- `.raw_rbsp_np` — H.264 / H.265 / H.266 `Sps` / `Pps` / `Vps` /
  `SliceHeaderLight`
- `.raw_np` — `Av1SequenceHeader`, `Av1FrameHeaderLight` (the field is
  named `raw`, not `raw_rbsp`)

These accessors are **read-only** views — `np.frombuffer` sets
`writeable=False` on Python `bytes`. Mutating attempts raise
`ValueError: assignment destination is read-only` by design.

##### Snapshot vs zero-copy

Each `.payload_np` / `.raw_rbsp_np` / `.raw_np` access materializes a
fresh Python `bytes` from Rust-owned storage (one copy), then NumPy
views that bytes object with no further copy. Per-access cost is
`O(payload_length)`; the view itself is a true zero-copy view over the
bytes object, but the bytes object is freshly allocated each time. For
repeated access on the same frame/NAL, cache the result manually:

```python
arr = nal.payload_np  # one copy from Rust
# use `arr` repeatedly — no further copy
```

A future plan may implement the Python buffer protocol directly on the
Rust types, eliminating the bytes copy. This is non-trivial because
each of the ~15 PyClass types would need `__getbuffer__` /
`__releasebuffer__` magic methods over stable Rust-owned storage.
Tracked as a v2 optimization.

For users who don't want the `.payload_np` indirection, the snapshot
is one line of stdlib NumPy:

```python
import numpy as np
arr = np.frombuffer(nal.payload, dtype=np.uint8)
```

Both forms are equivalent.

#### Common recipes

##### Plot platform altitude over time

```python
df = tstrans.pandas.klv_to_dataframe(uas_records)
df["sensor_alt_m"].plot()
# Or, if you want the framed-scene centre instead of the sensor itself:
df["frame_center_elev_m"].plot()
```

##### Filter Sample events by codec

```python
df = tstrans.pandas.events_to_dataframe(events)
h264_samples = df[(df.kind == "Sample") & (df.codec == "H264")]
```

##### NAL type histogram across an entire capture

```python
all_nals = []
for ev in events:
    if type(ev).__name__ == "_VideoEvent":
        all_nals.extend(ev.parse())  # opt-in split of each AU into NAL units
df = tstrans.pandas.nals_to_dataframe(all_nals)
df.nal_type_name.value_counts().plot.bar()
```

##### Audio frame-length over byte offset

```python
frames = list(parse_aac_frames(buf))
df = tstrans.pandas.audio_frames_to_dataframe(frames)
df.set_index("byte_offset")["frame_length_bytes"].plot()
```

#### Troubleshooting

**`TypeError: klv_to_dataframe requires homogeneous record types`** — your
input mixes ST sets (e.g. `UasDatalinkLs` + `SecurityLs`). Split into
per-set lists:

```python
from tstrans.klv import UasDatalinkLs, SecurityLs
uas = [r for r in records if isinstance(r, UasDatalinkLs)]
sec = [r for r in records if isinstance(r, SecurityLs)]
df_uas = tstrans.pandas.klv_to_dataframe(uas)
df_sec = tstrans.pandas.klv_to_dataframe(sec)
```

**KLV DataFrame falls back to `RangeIndex` instead of `DatetimeIndex`** —
none of your records had a populated timestamp. Common with legacy
ST 0102 SecurityLs (no internal timestamp) or partial captures whose
records pre-date the precision-timestamp tag.

**`field_errors` looks empty / non-empty unexpectedly** — lenient KLV
decode (the default) keeps a per-record `field_errors` list of
`KlvFieldError` entries for tags that failed to parse. The DataFrame
collapses these to a `|`-joined string. Empty `field_errors` becomes
the empty string `""`, not `NaN`. If you need a boolean instead, use
`df.field_errors.astype(bool)`.

**`nal_count` is `NaN` on audio rows** — by design.
`audio_frames_to_dataframe` is the audio-frame adapter; `nal_count` is
populated only on video Sample rows. See the column table above.

**`byte_offset` doesn't match the absolute byte position I expected** —
the cumulative offset is a running sum of `frame_length_bytes` starting
at zero, so it represents the offset within the contiguous-frame slice
the adapter saw. For `*_with_resync` flows, gaps caused by skipped
garbage bytes between frames are NOT reflected. Use the resync API
output directly when absolute byte offsets matter.

## Where this binding differs from the Rust core

- **Pipeline-shell naming follows the C-ABI convention, not the Rust
  crate's.** The TS-bytes-through-transport shells are `Sender` /
  `Receiver` (e.g. `tstrans.srt.Sender`) and the raw transports are
  `Transport` / `RecvTransport` (e.g. `tstrans.udp.Transport`) — there is
  no `RawSender` / `RawReceiver`. The composite shells keep the qualified
  `MuxSender` / `DemuxReceiver` names.
- **HLS is experimental and not in published wheels.** `tstrans.hls`
  imports only in a source build compiled with `--features hls`; from a
  PyPI wheel `import tstrans.hls` raises `ImportError`.
- **RIST is excluded from the Windows wheel.** The Linux and macOS wheels
  bundle librist; the Windows wheel does not. (UDP / TCP / RTP / SRT ship
  on every platform.)
- **RTSP passwords never round-trip to Python.** `BasicAuth` /
  `DigestAuth` hold the password in Rust memory; only `user` / `realm`
  (and `DigestAuth.algorithm`) are readable. `RtspServerConfig` TLS fields
  are forward-compat — setting them raises `RtspError(TLS)` at `start()`.
- **Subtitle Mux API is dataclass-driven.** Rust uses struct-variant
  enums for subtitle codec config; Python wraps each variant as a
  separate dataclass (`DvbSubtitlingConfig`, `DvbTeletextConfig`,
  `Cea708StandaloneConfig`, `WebVttInTsConfig`).
- **`add_subtitle()` and `push_subtitle*()` release the GIL.** Added in
  plan #96 Wave C.
- **No bindings for low-level `mpegts::demux::low_level::*`.** The Rust
  core exposes extension points there; the Python wrap omits them
  (would need PyO3 wrapping of trait objects).
- **Optional `[pandas]` extra is opt-in.** The `tstrans.pandas`
  submodule is only available if `pip install tstrans[pandas]` was
  used. Importing without the extra raises `ImportError` with a clear
  message.

(For pandas + NumPy specifics, see the
[Pandas + NumPy adapters](#pandas--numpy-adapters) sub-section under
"Language-specific gotchas" above.)

## Design

See [docs/specs/2026-05-22-tst-py-design.md](../../docs/specs/2026-05-22-tst-py-design.md)
(at parent-level project tree, outside the published repo).

## Roadmap

The full surface has shipped — offline file I/O, typed KLV decode /
encode, codec parsers, pandas / NumPy adapters, the UDP / TCP / RTP / RTSP /
SRT / RIST transports, and `tstrans.pipeline.Pairer`. Wheels publish to PyPI on tagged releases; the first PyPI release is v0.2.0. Remaining items are incremental: a
zero-copy Python-buffer-protocol path for the NumPy accessors (today each
access copies once — see [Snapshot vs zero-copy](#snapshot-vs-zero-copy)),
and graduating the experimental HLS publisher into the published wheels.
