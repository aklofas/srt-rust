"""Type stubs for `tstrans.rtp` — RTP + RTSP bindings.

Mirrors the Wave A / Wave B public surface exported from
`bindings/python/python/tstrans/rtp.py`. Continues `py.typed` discipline.
mypy --strict clean.

The PyClass-backed types (Sender, Receiver, RtspClient, RtspSession,
RtspServer, MountHandle, etc.) live in `_native.rtp` (Rust source under
`bindings/python/src/rtp/`). The pure-Python dataclass `RtspServerConfig`
lives in `rtp.py`.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import (
    Any,
    Callable,
    Iterator,
    List,
    Literal,
    Optional,
    Tuple,
    Type,
    Union,
)

# Cross-module types are imported from the `tstrans.mpegts` stub, which
# now ships a sibling `.pyi` declaring these as real classes.
from tstrans.mpegts import (
    AudioStreamHandle,
    DataStreamHandle,
    DemuxerConfig,
    DemuxEvent,
    KlvStreamHandle,
    MuxerProgramConfig,
    MuxerStats,
    Pts90khz,
    SubtitleStreamHandle,
    VideoStreamHandle,
)

# A bytes-like input — `bytes`, `bytearray`, `memoryview`, NumPy uint8,
# or any object implementing the buffer protocol. Concrete extraction
# happens in Rust via a two-path fast/fallback pattern (audit #10).
BytesLike = Union[bytes, bytearray, memoryview, Any]

__all__: list[str]

# ---------------------------------------------------------------------------
# T20 — RTP transport (Sender / Receiver / SocketStats / CancelHandle)
# ---------------------------------------------------------------------------


class SocketStats:
    """Frozen wire-level statistics snapshot. Mirror of
    `tst_core::transport::SocketStats`. All fields are integer-valued.

    `RtpTransport` populates `bytes_sent` / `packets_sent` only in
    Phase 1; `RtpRecvTransport` populates the receive-side counters.
    RTCP-derived fields (`rtt_us`, `packets_lost_*`) stay zero until
    RTCP RR/SR ingest is wired through the receiver.
    """

    rtt_us: int
    send_bandwidth_bps: int
    recv_bandwidth_bps: int
    link_bandwidth_bps: int
    bytes_sent: int
    packets_sent: int
    bytes_received: int
    packets_received: int
    bytes_lost_recv: int
    packets_lost_recv: int
    packets_lost_send: int
    packets_retransmitted: int
    packets_dropped_send: int
    packets_dropped_recv: int
    send_buffer_packets: int
    recv_buffer_packets: int

    def __repr__(self) -> str: ...


class CancelHandle:
    """Transport-side cancel handle for `Sender` / `Receiver`. Calling
    `.cancel()` wakes a thread parked in `.send()` / `.recv()` within
    ~100 ms; that call returns `RtpError(kind=CANCELLED)`.
    """

    def cancel(self) -> None: ...
    def __repr__(self) -> str: ...


class Sender:
    """Python RTP sender wrapping `tst_rtp::RtpTransport`.

    `pkt_size` overrides the UDP datagram size (RTP header + TS payload).
    `ssrc` pins the RTP synchronization source identifier; when omitted
    the transport picks a random one.
    """

    def __init__(
        self,
        url: str,
        *,
        pkt_size: int = ...,
        ssrc: Optional[int] = ...,
    ) -> None: ...
    def send(self, ts_bytes: BytesLike) -> None: ...
    def stats(self) -> SocketStats: ...
    def cancel_handle(self) -> CancelHandle: ...
    def close(self) -> None: ...
    def __enter__(self) -> Sender: ...
    def __exit__(
        self,
        exc_type: Optional[Type[BaseException]],
        exc_value: Optional[BaseException],
        traceback: Optional[Any],
    ) -> bool: ...
    def __repr__(self) -> str: ...


class Receiver:
    """Python RTP receiver wrapping `tst_rtp::RtpRecvTransport`.

    Binds to `url` (literal IP:port). For multicast URLs, joins the
    group automatically. `pkt_size` sizes the recv scratch buffer.
    """

    def __init__(self, url: str, *, pkt_size: int = ...) -> None: ...
    def recv(self) -> bytes: ...
    def stats(self) -> SocketStats: ...
    def cancel_handle(self) -> CancelHandle: ...
    def close(self) -> None: ...
    def __enter__(self) -> Receiver: ...
    def __exit__(
        self,
        exc_type: Optional[Type[BaseException]],
        exc_value: Optional[BaseException],
        traceback: Optional[Any],
    ) -> bool: ...
    def __repr__(self) -> str: ...


# ---------------------------------------------------------------------------
# T21 — RTSP client (enums, auth dataclasses, config, stats, session)
# ---------------------------------------------------------------------------


class RtspVersion:
    """Wire-time RTSP version preference (IntEnum-shaped PyClass).

    Mirrors `tst_rtp::RtspVersion` — only the 1.0 / 2.0 split matters
    at the SETUP / PLAY layer.
    """

    V1_0: RtspVersion
    V2_0: RtspVersion


class TransportPref:
    """Transport preference at SETUP time (IntEnum-shaped PyClass).

    `AUTO` = UDP-first with TCP fallback on 461 Unsupported Transport.
    `UDP` / `TCP` force a single transport (no fallback).
    """

    AUTO: TransportPref
    UDP: TransportPref
    TCP: TransportPref


class DigestAlgorithm:
    """Digest authentication algorithm selector (IntEnum-shaped PyClass).

    `MD5` — RFC 7616 §3.4 with `algorithm=MD5`.
    `SHA256` — RFC 7616 §3.4 with `algorithm=SHA-256`.
    """

    MD5: DigestAlgorithm
    SHA256: DigestAlgorithm


class BasicAuth:
    """HTTP Basic auth credentials per RFC 7617. Frozen.

    Password held in Rust memory and never re-exposed to Python; only
    `user` and `realm` are readable via getters. `realm` is optional
    and only used when this credential is passed as
    `RtspServerConfig.auth` (server-side challenge); client-side use
    reads the realm from the peer's 401.
    """

    def __init__(
        self,
        user: str,
        password: str,
        realm: str | None = None,
    ) -> None: ...
    @property
    def user(self) -> str: ...
    @property
    def realm(self) -> str | None: ...
    def __repr__(self) -> str: ...


class DigestAuth:
    """HTTP Digest auth credentials per RFC 7616 (MD5 + SHA-256) and
    RFC 2617 (legacy MD5). Frozen.

    Password held in Rust memory and never re-exposed to Python.
    `realm` is optional; same server-side-config semantics as
    `BasicAuth.realm`.
    """

    def __init__(
        self,
        user: str,
        password: str,
        algorithm: DigestAlgorithm = ...,
        realm: str | None = None,
    ) -> None: ...
    @property
    def user(self) -> str: ...
    @property
    def algorithm(self) -> DigestAlgorithm: ...
    @property
    def realm(self) -> str | None: ...
    def __repr__(self) -> str: ...


class RtspClientConfig:
    """RTSP client connection configuration. Frozen PyClass dataclass.

    `auth` is one of `BasicAuth`, `DigestAuth`, or `None`.
    `transport_pref` controls UDP/TCP selection at SETUP.
    `tls_root_certs_pem` is a PEM bundle for `rtsps://` connections.
    """

    def __init__(
        self,
        url: str,
        *,
        auth: Optional[Union[BasicAuth, DigestAuth]] = ...,
        transport_pref: TransportPref = ...,
        rtcp: bool = ...,
        tls_root_certs_pem: Optional[bytes] = ...,
        keepalive: bool = ...,
        rtsp_version: RtspVersion = ...,
    ) -> None: ...
    @property
    def url(self) -> str: ...
    @property
    def auth(self) -> Optional[Union[BasicAuth, DigestAuth]]: ...
    @property
    def transport_pref(self) -> TransportPref: ...
    @property
    def rtcp(self) -> bool: ...
    @property
    def tls_root_certs_pem(self) -> Optional[bytes]: ...
    @property
    def keepalive(self) -> bool: ...
    @property
    def rtsp_version(self) -> RtspVersion: ...
    def __repr__(self) -> str: ...


class RtspStats:
    """RTSP session stats snapshot. RTCP fields populated only when the
    session is in PLAY and the server has sent at least one RR / SR.
    Frozen.
    """

    @property
    def rr_packets_received(self) -> int: ...
    @property
    def sr_packets_received(self) -> int: ...
    @property
    def rr_packets_sent(self) -> int: ...
    @property
    def sr_packets_sent(self) -> int: ...
    @property
    def interarrival_jitter_us(self) -> int: ...
    @property
    def fraction_lost_q8(self) -> int: ...
    def __repr__(self) -> str: ...


class RtspCancelHandle:
    """RTSP control-plane cancel handle. Frozen. Flipping `.cancel()`
    breaks any in-flight `connect` / `pause` / `play` / `teardown` out
    of blocking I/O at the next poll (typically <100 ms).
    """

    def cancel(self) -> None: ...
    def is_canceled(self) -> bool: ...
    def __repr__(self) -> str: ...


class RtspClient:
    """Static facade. `connect(config)` runs OPTIONS / DESCRIBE / SETUP
    / PLAY and returns a live `RtspSession`.
    """

    @staticmethod
    def connect(config: RtspClientConfig) -> RtspSession: ...


class RtspSession:
    """Live RTSP session — server is in PLAY state. Methods drive
    `pause` / `play` / `teardown` and expose RTCP-derived stats.
    """

    def pause(self) -> None: ...
    def play(self) -> None: ...
    def teardown(self) -> None: ...
    def cancel_handle(self) -> RtspCancelHandle: ...
    def stats(self) -> RtspStats: ...
    def into_demux_receiver(
        self,
        demux_config: Optional[DemuxerConfig] = ...,
    ) -> DemuxReceiver: ...
    def is_torn_down(self) -> bool: ...
    def __enter__(self) -> RtspSession: ...
    def __exit__(
        self,
        exc_type: Optional[Type[BaseException]],
        exc_value: Optional[BaseException],
        traceback: Optional[Any],
    ) -> bool: ...


# ---------------------------------------------------------------------------
# T22 — RTSP server (RtspServer + MountHandle + stats)
# ---------------------------------------------------------------------------


class RtspServerCancelHandle:
    """Cross-thread hard-cancel handle for an `RtspServer`. Frozen.

    Calling `.cancel()` aborts every in-flight session at its next poll
    boundary, bypassing the graceful Notice-5402 path.
    """

    def cancel(self) -> None: ...
    def is_cancelled(self) -> bool: ...
    def __repr__(self) -> str: ...


class ServerStats:
    """Frozen snapshot of aggregate `RtspServer` stats."""

    @property
    def active_sessions(self) -> int: ...
    @property
    def total_rtp_packets_sent(self) -> int: ...
    @property
    def total_rtp_bytes_sent(self) -> int: ...
    @property
    def mounts(self) -> int: ...
    def __repr__(self) -> str: ...


class MountStats:
    """Frozen snapshot of per-mount stats."""

    @property
    def bytes_pushed(self) -> int: ...
    @property
    def packets_pushed(self) -> int: ...
    @property
    def peer_count(self) -> int: ...
    @property
    def frames_dropped_total(self) -> int: ...
    def __repr__(self) -> str: ...


class MountHandle:
    """Public mount surface returned by `RtspServer.add_unicast_mount` /
    `add_multicast_mount`. Cloneable (Arc-shared); multiple holders can
    push from different threads.

    All `push_*` methods release the GIL via `py.allow_threads()` so
    concurrent Python threads can run while the muxer/fanout work
    proceeds. `pts` is keyword-only per Wave C normalization.
    """

    # Identity / introspection.
    def mount_path(self) -> str: ...
    def peer_count(self) -> int: ...
    def mount_kind(self) -> Literal["unicast", "multicast", "unknown"]: ...
    def stats(self) -> MountStats: ...

    # Push surface — single stream (lone configured stream of each kind).
    def push_video(
        self,
        nal: BytesLike,
        *,
        pts: Pts90khz,
        key_frame: bool = ...,
    ) -> None: ...
    def push_klv(
        self,
        klv: BytesLike,
        *,
        pts: Pts90khz,
        metadata_service_id: int = ...,
    ) -> None: ...
    def push_audio(self, frames: BytesLike, *, pts: Pts90khz) -> None: ...
    def push_subtitle(self, payload: BytesLike, *, pts: Pts90khz) -> None: ...

    # Push surface — multi-stream (explicit handle).
    def push_video_to(
        self,
        handle: VideoStreamHandle,
        nal: BytesLike,
        *,
        pts: Pts90khz,
        key_frame: bool = ...,
    ) -> None: ...
    def push_klv_to(
        self,
        handle: KlvStreamHandle,
        klv: BytesLike,
        *,
        pts: Pts90khz,
        metadata_service_id: int = ...,
    ) -> None: ...
    def push_audio_to(
        self,
        handle: AudioStreamHandle,
        frames: BytesLike,
        *,
        pts: Pts90khz,
    ) -> None: ...
    def push_subtitle_to(
        self,
        handle: SubtitleStreamHandle,
        payload: BytesLike,
        *,
        pts: Pts90khz,
    ) -> None: ...

    # Stream-handle accessors (first-of-kind + all-of-kind).
    def video_handle(self) -> Optional[VideoStreamHandle]: ...
    def klv_handle(self) -> Optional[KlvStreamHandle]: ...
    def audio_handle(self) -> Optional[AudioStreamHandle]: ...
    def subtitle_handle(self) -> Optional[SubtitleStreamHandle]: ...
    def video_handles(self) -> List[VideoStreamHandle]: ...
    def klv_handles(self) -> List[KlvStreamHandle]: ...
    def audio_handles(self) -> List[AudioStreamHandle]: ...
    def subtitle_handles(self) -> List[SubtitleStreamHandle]: ...

    # Lifecycle.
    def flush(self) -> None: ...
    def reset_stats(self) -> None: ...
    def __repr__(self) -> str: ...


@dataclass(frozen=True)
class RtspServerConfig:
    """Pure-Python frozen dataclass — configuration for `RtspServer.start`.

    `auth` accepts the same `BasicAuth` / `DigestAuth` PyClass instances
    that T21 introduced for the client side.

    `tls_cert_pem` / `tls_key_pem` are forward-compat fields; setting
    either today raises `RtspError(TLS)` at `start()` because the
    tst-rtp `tls` feature isn't wired through tst-py yet.
    """

    bind_addr: str = ...
    auth: Optional[Union[BasicAuth, DigestAuth]] = ...
    max_sessions: int = ...
    session_timeout_secs: int = ...
    fanout_capacity: int = ...
    graceful_shutdown_drain_ms: int = ...
    tls_cert_pem: Optional[bytes] = ...
    tls_key_pem: Optional[bytes] = ...

    def __post_init__(self) -> None: ...


class RtspServer:
    """Sync RTSP server. Construct via `RtspServer.start(config)`.

    The underlying `tst_rtp::RtspServer` owns a tokio Runtime that lives
    for the server's lifetime; `__exit__` (or explicit `.stop()`) sends
    an RFC 7826 §13.5.1 Notice 5402 ("Server-Initiated TEARDOWN") to
    each active session before closing.
    """

    @staticmethod
    def start(config: RtspServerConfig) -> RtspServer: ...
    def add_unicast_mount(
        self,
        path: str,
        program_config: MuxerProgramConfig,
    ) -> MountHandle: ...
    def add_multicast_mount(
        self,
        path: str,
        group: str,
        port: int,
        *,
        ttl: int = ...,
        iface: Optional[str] = ...,
        program_config: MuxerProgramConfig,
    ) -> MountHandle: ...
    def stats(self) -> ServerStats: ...
    def local_addr(self) -> Optional[str]: ...
    def stop(self, *, drain_ms: int = ...) -> None: ...
    def cancel_handle(self) -> RtspServerCancelHandle: ...
    def __enter__(self) -> RtspServer: ...
    def __exit__(
        self,
        exc_type: Optional[Type[BaseException]] = ...,
        exc_value: Optional[BaseException] = ...,
        traceback: Optional[Any] = ...,
    ) -> bool: ...
    def __repr__(self) -> str: ...


# ---------------------------------------------------------------------------
# T23 — MuxSender + DemuxReceiver convenience wrappers (Wave B).
#
# The Rust implementation is being authored in a parallel worktree; the
# stub here mirrors the surface declared in the plan
# (`docs/plans/2026-05-26-tst-rtp-phase-4-binding-exposure.md`,
# Task 23 lines 2456-2470). The merge phase will reconcile if the
# implementation diverges.
# ---------------------------------------------------------------------------


class MuxSender:
    """Convenience wrapper — `Sender` + `Muxer` constructed together.

    Single-call constructor; push methods accept any bytes-like input
    (`bytes`, `bytearray`, `memoryview`, NumPy `uint8`). `pts` is
    keyword-only on every push method. The GIL is released around the
    muxer + transport work via `py.allow_threads()`.
    """

    def __init__(
        self,
        url: str,
        program_config: MuxerProgramConfig,
        *,
        pkt_size: int = ...,
    ) -> None: ...

    # Push surface — single stream.
    def push_video(
        self,
        nal: BytesLike,
        *,
        pts: Pts90khz,
        key_frame: bool = ...,
    ) -> None: ...
    def push_klv(
        self,
        klv: BytesLike,
        *,
        pts: Pts90khz,
        metadata_service_id: int = ...,
    ) -> None: ...
    def push_audio(self, frames: BytesLike, *, pts: Pts90khz) -> None: ...
    def push_subtitle(self, payload: BytesLike, *, pts: Pts90khz) -> None: ...
    def push_data(self, data: BytesLike, *, pts: Pts90khz) -> None: ...

    # Push surface — multi-stream variants.
    def push_video_to(
        self,
        handle: VideoStreamHandle,
        nal: BytesLike,
        *,
        pts: Pts90khz,
        key_frame: bool = ...,
    ) -> None: ...
    def push_klv_to(
        self,
        handle: KlvStreamHandle,
        klv: BytesLike,
        *,
        pts: Pts90khz,
        metadata_service_id: int = ...,
    ) -> None: ...
    def push_audio_to(
        self,
        handle: AudioStreamHandle,
        frames: BytesLike,
        *,
        pts: Pts90khz,
    ) -> None: ...
    def push_subtitle_to(
        self,
        handle: SubtitleStreamHandle,
        payload: BytesLike,
        *,
        pts: Pts90khz,
    ) -> None: ...
    def push_data_to(
        self,
        handle: DataStreamHandle,
        data: BytesLike,
        *,
        pts: Pts90khz,
    ) -> None: ...

    # Stream handle accessors.
    def video_handle(self) -> Optional[VideoStreamHandle]: ...
    def klv_handle(self) -> Optional[KlvStreamHandle]: ...
    def audio_handle(self) -> Optional[AudioStreamHandle]: ...
    def subtitle_handle(self) -> Optional[SubtitleStreamHandle]: ...
    def data_handle(self) -> Optional[DataStreamHandle]: ...

    def stats(self) -> Tuple[SocketStats, MuxerStats]: ...
    def cancel_handle(self) -> CancelHandle: ...
    def close(self) -> None: ...
    def __enter__(self) -> MuxSender: ...
    def __exit__(
        self,
        exc_type: Optional[Type[BaseException]],
        exc_value: Optional[BaseException],
        traceback: Optional[Any],
    ) -> bool: ...
    def __repr__(self) -> str: ...


class DemuxReceiver:
    """Convenience wrapper — `Receiver` + `Demuxer` constructed together.

    Iterating yields `tstrans.mpegts.DemuxEvent` subclass instances —
    no duplicated event type hierarchy. `__next__` blocks (releases the
    GIL) on the underlying transport read.

    `stats()` returns a tuple `(SocketStats, Any)` — the second
    element is the demuxer-side stats snapshot whose concrete type is
    pending T23's final pick (no `DemuxerStats` PyClass exists in
    `tstrans.mpegts` yet).
    """

    def __init__(
        self,
        url: str,
        *,
        demux_config: Optional[DemuxerConfig] = ...,
    ) -> None: ...
    def __iter__(self) -> Iterator[DemuxEvent]: ...
    def __next__(self) -> DemuxEvent: ...
    # Register a fan-out callback invoked with every 188-byte TS packet
    # (as `bytes`) BEFORE demuxing, in registration order. If the
    # callback raises, the exception is re-raised fail-loud from the next
    # `__next__` and iteration stops. Append-only for the receiver's life.
    def add_byte_sink(self, callback: Callable[[bytes], None]) -> None: ...
    def stats(self) -> Tuple[SocketStats, Any]: ...
    def cancel_handle(self) -> CancelHandle: ...
    def close(self) -> None: ...
    def __enter__(self) -> DemuxReceiver: ...
    def __exit__(
        self,
        exc_type: Optional[Type[BaseException]],
        exc_value: Optional[BaseException],
        traceback: Optional[Any],
    ) -> bool: ...
    def __repr__(self) -> str: ...


# ---------------------------------------------------------------------------
# @overload sanity — RtspClientConfig accepts BasicAuth | DigestAuth | None.
# A single typed parameter handles all three forms; explicit overloads
# would be redundant given the Union annotation.
# ---------------------------------------------------------------------------
