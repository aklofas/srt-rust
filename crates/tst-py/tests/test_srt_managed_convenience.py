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
            sender.push_video(
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
            s.push_video(NAL_IDR, pts=Pts90khz.from_raw(0))
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
        sender.push_video(NAL_IDR, pts=Pts90khz.from_raw(0))
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
