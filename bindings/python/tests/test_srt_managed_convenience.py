"""Tests for `tstrans.srt.ManagedMuxSender` + `tstrans.srt.ManagedDemuxReceiver`
(Wave C Task 8).

Loopback-only tests against a libsrt caller<->listener pair. Each test
spawns a listener-side worker on a background thread before the
caller-side construct connects; the test joins the worker at the end.
No external network is required.

Reconnect-exercising tests are intentionally minimal — driving a real
reconnect on a loopback SRT pair is brittle (the listener side has to
restart on the same port within the policy's max_attempts × backoff
window, and the caller's gap-buffer drain has to fire before the test
times out). These tests verify the surface compiles and behaves
correctly on the happy path; the underlying reconnect logic is
covered by `crates/tst-pipeline/src/managed_demux_receiver.rs` unit
tests against a scripted RecvTransport.
"""

from __future__ import annotations

import socket
import threading
import time
from typing import Optional, Tuple

import pytest

import tstrans
import tstrans.srt
from tstrans.exceptions import SrtError, SrtErrorKind
from tstrans.mpegts import (
    DataStreamHandle,
    DemuxEvent,
    DemuxerConfig,
    MuxerProgramConfigBuilder,
    Pts90khz,
    StrictMode,
    VideoCodec,
)
from tstrans.srt import (
    BackoffStrategy,
    ManagedDemuxReceiver,
    ManagedMuxSender,
    ReconnectPolicy,
)


# --------------------------------------------------------------------------- #
# Helpers                                                                     #
# --------------------------------------------------------------------------- #


def _free_tcp_port() -> int:
    """Ask the OS for an ephemeral TCP port, then release it."""
    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


def _video_only_program() -> object:
    return (
        MuxerProgramConfigBuilder(1, 0x100)
        .add_video(0x101, VideoCodec.H264)
        .build()
    )


def _video_data_program() -> object:
    """Video + one private data stream (user-private stream_type 0xF0)."""
    return (
        MuxerProgramConfigBuilder(1, 0x100)
        .add_video(0x101, VideoCodec.H264)
        .add_data(0x1F0, 0xF0, carries_pts=True)
        .build()
    )


def _fast_policy() -> ReconnectPolicy:
    """ReconnectPolicy with zero-wait constant backoff so tests don't sleep."""
    return ReconnectPolicy(
        max_attempts=3,
        backoff=BackoffStrategy.constant(ms=0),
    )


def _make_managed_pair(
    port: int,
) -> Tuple[ManagedMuxSender, ManagedDemuxReceiver]:
    """Spawn a listener-mode ManagedDemuxReceiver on a background
    thread; once it's accepting, connect a caller-mode ManagedMuxSender.

    Returns (mux_sender, demux_receiver). Callers must close both.
    """
    listener_url = f"srt://:{port}?mode=listener"
    caller_url = f"srt://127.0.0.1:{port}?mode=caller"

    rx_box: list[ManagedDemuxReceiver] = []
    rx_err: list[BaseException] = []

    def accept_worker() -> None:
        try:
            r = ManagedDemuxReceiver.from_url(listener_url, policy=_fast_policy())
            rx_box.append(r)
        except BaseException as exc:  # noqa: BLE001
            rx_err.append(exc)

    t = threading.Thread(target=accept_worker, daemon=True)
    t.start()
    # libsrt's bind+listen is fast but not instant; sleep before
    # connecting from the main thread to avoid racing the listener.
    time.sleep(0.1)
    sender = ManagedMuxSender.from_url(
        caller_url, _video_only_program(), policy=_fast_policy()
    )
    t.join(timeout=5.0)
    if rx_err:
        sender.close()
        raise rx_err[0]
    if not rx_box:
        sender.close()
        raise RuntimeError("ManagedDemuxReceiver listener thread did not accept within 5 s")
    return sender, rx_box[0]


# Minimal Annex-B IDR NAL.
NAL_IDR = b"\x00\x00\x00\x01\x65\xBB"


# --------------------------------------------------------------------------- #
# Module surface                                                              #
# --------------------------------------------------------------------------- #


def test_managed_module_re_exports() -> None:
    """`tstrans.srt.{ManagedMuxSender,ManagedDemuxReceiver}` exposed."""
    assert tstrans.srt.ManagedMuxSender is not None
    assert tstrans.srt.ManagedDemuxReceiver is not None
    assert "ManagedMuxSender" in tstrans.srt.__all__
    assert "ManagedDemuxReceiver" in tstrans.srt.__all__


# --------------------------------------------------------------------------- #
# Test 1: basic loopback round-trip                                           #
# --------------------------------------------------------------------------- #


def test_managed_loopback_round_trip() -> None:
    """End-to-end: ManagedMuxSender(caller) <-> ManagedDemuxReceiver(listener).
    Push a burst of video NALs; verify the receiver observes at least one
    Video or ProgramMap event."""
    port = _free_tcp_port()
    sender, receiver = _make_managed_pair(port)

    events: list[object] = []
    consumer_err: list[BaseException] = []

    def consumer() -> None:
        try:
            for ev in receiver:
                events.append(ev)
                if isinstance(ev, DemuxEvent.Video):
                    break
        except BaseException as exc:  # noqa: BLE001
            consumer_err.append(exc)

    t = threading.Thread(target=consumer, daemon=True)
    t.start()
    # Park the receiver inside recv_event before pushing.
    time.sleep(0.2)
    try:
        for i in range(32):
            sender.send_video(
                NAL_IDR, pts=Pts90khz.from_raw(i * 3000), key_frame=(i % 4 == 0)
            )
        # Drain in-flight before close.
        time.sleep(0.3)
    finally:
        sender.close()
    t.join(timeout=5.0)
    receiver.close()
    saw_video_or_pmt = any(
        isinstance(e, (DemuxEvent.Video, DemuxEvent.ProgramMap)) for e in events
    )
    if not saw_video_or_pmt and consumer_err:
        pytest.fail(f"consumer raised before any event: {consumer_err}")
    assert saw_video_or_pmt, (
        f"expected at least one Video or ProgramMap event; got: "
        f"{[type(e).__name__ for e in events]}"
    )


# --------------------------------------------------------------------------- #
# Test 2: policy plumbing + reconnect_attempts initial value                  #
# --------------------------------------------------------------------------- #


def test_managed_mux_sender_accepts_policy_and_reports_attempts() -> None:
    """`ManagedMuxSender.from_url(policy=ReconnectPolicy(...))` accepts a
    custom policy; `reconnect_attempts()` returns 0 on the happy path
    (no reconnect has fired yet)."""
    port = _free_tcp_port()
    sender, receiver = _make_managed_pair(port)
    try:
        # The initial connect is live; the factory has not been invoked
        # yet (it's only called on Broken/Closed from the inner socket).
        assert sender.reconnect_attempts() == 0
        # Mirror on the receiver side.
        assert receiver.reconnect_attempts() == 0
        # repr includes the counter for diagnostics.
        assert "reconnect_attempts=0" in repr(sender)
        assert "reconnect_attempts=0" in repr(receiver)
    finally:
        sender.close()
        receiver.close()


def test_managed_mux_sender_reconnect_stats_healthy_link() -> None:
    """On a healthy link (no break yet), `reconnect_stats()` must return
    the typed `ManagedTransportStats` object with all counters at zero
    and `reconnecting` False."""
    port = _free_tcp_port()
    sender, receiver = _make_managed_pair(port)
    try:
        stats = sender.reconnect_stats()
        assert isinstance(stats, tstrans.srt.ManagedTransportStats)
        assert stats.reconnect_attempts == 0
        assert stats.reconnect_successes == 0
        assert stats.gap_len == 0
        assert stats.gap_messages_dropped == 0
        assert stats.gap_bytes_dropped == 0
        assert stats.reconnecting is False
    finally:
        sender.close()
        receiver.close()


def test_managed_mux_sender_reconnect_stats_closed_raises() -> None:
    """`reconnect_stats()` on a closed sender raises `SrtError(CLOSED)`."""
    port = _free_tcp_port()
    sender, receiver = _make_managed_pair(port)
    sender.close()
    receiver.close()
    with pytest.raises(SrtError) as exc_info:
        sender.reconnect_stats()
    assert exc_info.value.kind == SrtErrorKind.CLOSED


# --------------------------------------------------------------------------- #
# Test 3: demux_config dataclass propagation                                  #
# --------------------------------------------------------------------------- #


def test_managed_demux_receiver_accepts_demux_config() -> None:
    """`ManagedDemuxReceiver.from_url(demux_config=...)` accepts a
    DemuxerConfig dataclass and routes it through
    `crate::mpegts::build_demuxer_config`."""
    port = _free_tcp_port()
    listener_url = f"srt://:{port}?mode=listener"
    caller_url = f"srt://127.0.0.1:{port}?mode=caller"
    cfg = DemuxerConfig(strict_mode=StrictMode.OFF)

    rx_box: list[ManagedDemuxReceiver] = []
    rx_err: list[BaseException] = []

    def accept_worker() -> None:
        try:
            r = ManagedDemuxReceiver.from_url(
                listener_url, demux_config=cfg, policy=_fast_policy()
            )
            rx_box.append(r)
        except BaseException as exc:  # noqa: BLE001
            rx_err.append(exc)

    t = threading.Thread(target=accept_worker, daemon=True)
    t.start()
    time.sleep(0.1)
    sender = ManagedMuxSender.from_url(
        caller_url, _video_only_program(), policy=_fast_policy()
    )
    t.join(timeout=5.0)
    if rx_err:
        sender.close()
        raise rx_err[0]
    assert rx_box, "ManagedDemuxReceiver did not accept under demux_config kwarg"
    sender.close()
    rx_box[0].close()


# --------------------------------------------------------------------------- #
# Test 4: context manager closes cleanly on both sides                        #
# --------------------------------------------------------------------------- #


def test_managed_context_manager_closes_cleanly() -> None:
    """`with ManagedMuxSender(...) as s: ...` and `with
    ManagedDemuxReceiver(...) as rx: ...` close the wrapped shell on
    exit; repeated close is a no-op."""
    port = _free_tcp_port()
    sender, receiver = _make_managed_pair(port)
    try:
        with sender as s:
            assert "open" in repr(s)
            s.send_video(NAL_IDR, pts=Pts90khz.from_raw(0))
        # After __exit__, sender is closed.
        assert "closed" in repr(sender)
        # Idempotent close.
        sender.close()
        sender.close()

        with receiver as rx:
            assert "open" in repr(rx)
        assert "closed" in repr(receiver)
        receiver.close()  # idempotent
    finally:
        # Defensive cleanup in case an assertion fired mid-with.
        sender.close()
        receiver.close()


# --------------------------------------------------------------------------- #
# Construction-time errors                                                    #
# --------------------------------------------------------------------------- #


def test_managed_mux_sender_wrong_mode_raises_config_invalid() -> None:
    """ManagedMuxSender.from_url requires ?mode=caller."""
    port = _free_tcp_port()
    with pytest.raises(SrtError) as exc_info:
        ManagedMuxSender.from_url(
            f"srt://127.0.0.1:{port}?mode=listener", _video_only_program()
        )
    assert exc_info.value.kind == SrtErrorKind.CONFIG_INVALID


def test_managed_mux_sender_bad_url_raises_config_invalid() -> None:
    with pytest.raises(SrtError) as exc_info:
        ManagedMuxSender.from_url("not-a-valid-url", _video_only_program())
    assert exc_info.value.kind == SrtErrorKind.CONFIG_INVALID


def test_managed_push_on_closed_sender_raises_closed() -> None:
    """Pushing on a closed ManagedMuxSender raises SrtError(CLOSED)."""
    port = _free_tcp_port()
    sender, receiver = _make_managed_pair(port)
    sender.close()
    receiver.close()
    with pytest.raises(SrtError) as exc_info:
        sender.send_video(NAL_IDR, pts=Pts90khz.from_raw(0))
    assert exc_info.value.kind == SrtErrorKind.CLOSED


# --------------------------------------------------------------------------- #
# DemuxReceiver iterator returns self                                         #
# --------------------------------------------------------------------------- #


def test_managed_demux_receiver_iter_returns_self() -> None:
    port = _free_tcp_port()
    sender, receiver = _make_managed_pair(port)
    try:
        assert iter(receiver) is receiver
    finally:
        sender.close()
        receiver.close()


# --------------------------------------------------------------------------- #
# push_data / push_data_to / data_handle                                      #
# --------------------------------------------------------------------------- #


def test_data_handle_none_for_video_only_program() -> None:
    """data_handle() returns None when no data stream is configured."""
    port = _free_tcp_port()
    sender, receiver = _make_managed_pair(port)
    try:
        assert sender.data_handle() is None
    finally:
        sender.close()
        receiver.close()


def test_data_handle_returns_handle_for_data_program() -> None:
    """data_handle() returns a DataStreamHandle when a data stream is
    configured in the program."""
    port = _free_tcp_port()
    listener_url = f"srt://:{port}?mode=listener"
    caller_url = f"srt://127.0.0.1:{port}?mode=caller"

    rx_box: list[ManagedDemuxReceiver] = []
    rx_err: list[BaseException] = []

    def accept_worker() -> None:
        try:
            r = ManagedDemuxReceiver.from_url(listener_url, policy=_fast_policy())
            rx_box.append(r)
        except BaseException as exc:  # noqa: BLE001
            rx_err.append(exc)

    t = threading.Thread(target=accept_worker, daemon=True)
    t.start()
    time.sleep(0.1)
    sender = ManagedMuxSender.from_url(
        caller_url, _video_data_program(), policy=_fast_policy()
    )
    t.join(timeout=5.0)
    if rx_err:
        sender.close()
        raise rx_err[0]
    receiver = rx_box[0]
    try:
        h = sender.data_handle()
        assert h is not None
        assert isinstance(h, DataStreamHandle)
    finally:
        sender.close()
        receiver.close()


def test_push_data_on_closed_sender_raises_closed() -> None:
    """push_data on a closed ManagedMuxSender raises SrtError(CLOSED)."""
    port = _free_tcp_port()
    listener_url = f"srt://:{port}?mode=listener"
    caller_url = f"srt://127.0.0.1:{port}?mode=caller"

    rx_box: list[ManagedDemuxReceiver] = []

    def accept_worker() -> None:
        try:
            r = ManagedDemuxReceiver.from_url(listener_url, policy=_fast_policy())
            rx_box.append(r)
        except BaseException:  # noqa: BLE001
            pass

    t = threading.Thread(target=accept_worker, daemon=True)
    t.start()
    time.sleep(0.1)
    sender = ManagedMuxSender.from_url(
        caller_url, _video_data_program(), policy=_fast_policy()
    )
    t.join(timeout=5.0)
    if rx_box:
        rx_box[0].close()
    sender.close()
    with pytest.raises(SrtError) as exc_info:
        sender.send_data(b"\x01\x02\x03", pts=Pts90khz.from_raw(0))
    assert exc_info.value.kind == SrtErrorKind.CLOSED


def test_push_data_round_trips_payload_fidelity() -> None:
    """End-to-end over a managed SRT loopback: push distinct payloads via
    both `push_data` (single-stream shorthand) and `push_data_to`
    (explicit handle); the receiver-side demuxer must surface them
    byte-faithfully as `UnknownSample` events on the data PID (0x1F0,
    stream_type 0xF0), with the pushed PTS preserved (carries_pts=True).

    The consumer thread only *collects* events — all fidelity assertions
    run on the main thread after draining, so a genuine payload/pts
    mismatch surfaces as a real test failure instead of being swallowed
    into a background-thread exception list."""
    port = _free_tcp_port()
    listener_url = f"srt://:{port}?mode=listener"
    caller_url = f"srt://127.0.0.1:{port}?mode=caller"

    DATA_PID = 0x1F0
    DATA_STREAM_TYPE = 0xF0
    PAYLOAD_SHORTHAND = b"\x01\x02\x03\x04shorthand-record"
    PAYLOAD_HANDLE = b"\x05\x06\x07\x08handle-record"
    # carries_pts=True → the pushed PTS round-trips verbatim (non-zero).
    PTS_BASE = 900_000
    PTS_STEP = 3000
    N = 24
    pushed_pts = {PTS_BASE + i * PTS_STEP for i in range(N)}
    # Key-frame video NAL so the demuxer reaches a ProgramMap and starts
    # surfacing samples promptly.
    NAL_IDR_DATA = b"\x00\x00\x00\x01\x65\xBB"

    rx_box: list[ManagedDemuxReceiver] = []
    rx_err: list[BaseException] = []

    def accept_worker() -> None:
        try:
            r = ManagedDemuxReceiver.from_url(listener_url, policy=_fast_policy())
            rx_box.append(r)
        except BaseException as exc:  # noqa: BLE001
            rx_err.append(exc)

    t = threading.Thread(target=accept_worker, daemon=True)
    t.start()
    time.sleep(0.1)
    sender = ManagedMuxSender.from_url(
        caller_url, _video_data_program(), policy=_fast_policy()
    )
    t.join(timeout=5.0)
    if rx_err:
        sender.close()
        raise rx_err[0]
    receiver = rx_box[0]

    data_samples: list[object] = []
    consumer_err: list[BaseException] = []

    def consumer() -> None:
        # Collect-only: append every UnknownSample, break once both
        # target payloads have been observed. No assertions here.
        try:
            seen_payloads: set[bytes] = set()
            for ev in receiver:
                if isinstance(ev, DemuxEvent.UnknownSample):
                    data_samples.append(ev)
                    seen_payloads.add(bytes(ev.payload))
                    if {PAYLOAD_SHORTHAND, PAYLOAD_HANDLE} <= seen_payloads:
                        break
        except BaseException as exc:  # noqa: BLE001
            consumer_err.append(exc)

    ct = threading.Thread(target=consumer, daemon=True)
    ct.start()
    # Park the receiver inside recv_event before pushing.
    time.sleep(0.2)
    try:
        data_h = sender.data_handle()
        assert data_h is not None
        for i in range(N):
            pts = Pts90khz.from_raw(PTS_BASE + i * PTS_STEP)
            sender.send_video(NAL_IDR_DATA, pts=pts, key_frame=(i % 4 == 0))
            sender.send_data(PAYLOAD_SHORTHAND, pts=pts)
            sender.send_data_to(data_h, PAYLOAD_HANDLE, pts=pts)
        # Give TSBPD time to release the buffered packets to the consumer.
        time.sleep(0.5)
    finally:
        # Close the RECEIVER first: its cancel handle fires while the
        # consumer is parked in recv_event (after it has drained the
        # buffered samples), unblocking it cleanly. Closing the sender
        # first would break the SRT link and make the managed receiver
        # attempt a reconnect, parking in a blocking re-accept the cancel
        # handle can't interrupt — a dropped/mismatched record would then
        # HANG the test instead of failing loud.
        receiver.close()
        ct.join(timeout=5.0)
        sender.close()

    if not data_samples and consumer_err:
        pytest.fail(f"consumer raised before any data sample: {consumer_err}")

    # ── Fidelity assertions (main thread) ──────────────────────────────
    assert data_samples, (
        "expected at least one UnknownSample on the data stream; got none "
        "(a regression dropping every data record would land here)"
    )
    payloads = {bytes(s.payload) for s in data_samples}
    assert PAYLOAD_SHORTHAND in payloads, (
        f"push_data payload not round-tripped; observed payloads={payloads!r}"
    )
    assert PAYLOAD_HANDLE in payloads, (
        f"push_data_to payload not round-tripped; observed payloads={payloads!r}"
    )
    # No corruption: every surfaced data payload is exactly one of the two
    # we pushed, on the right PID / stream_type, with a preserved PTS.
    for s in data_samples:
        assert bytes(s.payload) in (PAYLOAD_SHORTHAND, PAYLOAD_HANDLE), (
            f"unexpected/corrupt data payload: {bytes(s.payload)!r}"
        )
        assert s.stream.pid == DATA_PID
        assert s.stream_type == DATA_STREAM_TYPE
        # carries_pts=True ⇒ pts is one of the pushed (non-zero) values,
        # not the demuxer's no-PTS substitute of 0.
        assert s.pts.raw != 0
        assert s.pts.raw in pushed_pts
