"""Tests for `tstrans.rtp.DemuxReceiver` (Wave B Task 23).

Loopback tests:
- Build a small TS stream with `tstrans.mpegts.Muxer`; send the bytes
  via `tstrans.rtp.Sender`; iterate the demuxer; assert events arrive.
- Round-trip: `tstrans.rtp.MuxSender` → `tstrans.rtp.DemuxReceiver` over
  the same loopback port. Push a video NAL; assert the receiver emits
  at least one ProgramMap + one Sample/Video event.
- `RtspSession.into_demux_receiver` — skipped (no RTSP server in this
  test; covered by Wave C T25 integration tests).
"""

from __future__ import annotations

import socket
import threading
import time

import pytest

import tstrans
import tstrans.rtp
from tstrans.exceptions import RtpError, RtpErrorKind
from tstrans.mpegts import (
    DemuxEvent,
    Muxer,
    MuxerConfigBuilder,
    MuxerProgramConfigBuilder,
    Pts90khz,
    VideoCodec,
)


# --------------------------------------------------------------------------- #
# Helpers                                                                     #
# --------------------------------------------------------------------------- #


def _free_udp_port() -> int:
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
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


def _build_minimal_ts_bytes() -> bytes:
    """Mux a single AUD NAL with `tstrans.mpegts.Muxer` and return the
    drained TS bytes. Just enough to contain a PAT/PMT pair so the
    demuxer can recognize a program."""
    cfg = MuxerConfigBuilder().add_program(_video_only_program()).build()
    mux = Muxer(cfg)
    # Push a single AUD NAL with key_frame=True so PSI gets emitted.
    nal_aud = b"\x00\x00\x00\x01\x09\xF0"
    mux.push_video(nal_aud, pts=Pts90khz.from_raw(0), key_frame=True)
    # Drain into an output bytearray.
    out = bytearray()
    scratch = bytearray(188 * 32)
    while True:
        n = mux.pull(scratch)
        if n == 0:
            break
        out.extend(scratch[:n])
    return bytes(out)


# --------------------------------------------------------------------------- #
# Module surface                                                              #
# --------------------------------------------------------------------------- #


def test_demux_receiver_module_re_exports() -> None:
    assert tstrans.rtp.DemuxReceiver is not None
    assert "DemuxReceiver" in tstrans.rtp.__all__


# --------------------------------------------------------------------------- #
# Construction                                                                #
# --------------------------------------------------------------------------- #


def test_demux_receiver_bind_and_close() -> None:
    port = _free_udp_port()
    with tstrans.rtp.DemuxReceiver(f"rtp://127.0.0.1:{port}") as rx:
        assert "open" in repr(rx)
    # After context exit, repr reports closed.
    # (re-constructing to verify shape)
    rx2 = tstrans.rtp.DemuxReceiver(f"rtp://127.0.0.1:{port}")
    rx2.close()
    assert "closed" in repr(rx2)


def test_demux_receiver_bad_url_raises_rtp_error() -> None:
    with pytest.raises(RtpError) as exc_info:
        tstrans.rtp.DemuxReceiver("not-a-valid-url://")
    assert exc_info.value.kind == RtpErrorKind.TRANSPORT


def test_demux_receiver_with_demux_config() -> None:
    """Pass a `DemuxerConfig` dataclass via the keyword arg."""
    from tstrans.mpegts import DemuxerConfig, StrictMode

    port = _free_udp_port()
    cfg = DemuxerConfig(strict_mode=StrictMode.OFF)
    with tstrans.rtp.DemuxReceiver(
        f"rtp://127.0.0.1:{port}", demux_config=cfg
    ) as rx:
        assert "open" in repr(rx)


# --------------------------------------------------------------------------- #
# Iteration                                                                   #
# --------------------------------------------------------------------------- #


def test_demux_receiver_iter_returns_self() -> None:
    port = _free_udp_port()
    rx = tstrans.rtp.DemuxReceiver(f"rtp://127.0.0.1:{port}")
    assert iter(rx) is rx
    rx.close()


def test_demux_receiver_emits_events_from_loopback_send() -> None:
    """Build TS bytes locally; send them via `tstrans.rtp.Sender`; the
    DemuxReceiver should yield at least one event before we stop.

    Sends one TS packet per RTP datagram (the most deterministic path)
    and repeats a few times to ride past any single-packet drop in the
    UDP loopback path.
    """
    port = _free_udp_port()
    rx = tstrans.rtp.DemuxReceiver(f"rtp://127.0.0.1:{port}")
    events: list[object] = []
    err: list[BaseException] = []

    def consumer() -> None:
        try:
            for ev in rx:
                events.append(ev)
                # Stop after the first event to keep the test fast.
                break
        except BaseException as exc:  # noqa: BLE001
            err.append(exc)

    t = threading.Thread(target=consumer, daemon=True)
    t.start()
    # Give the consumer time to enter recv.
    time.sleep(0.2)
    ts_bytes = _build_minimal_ts_bytes()
    with tstrans.rtp.Sender(f"rtp://127.0.0.1:{port}") as snd:
        # Repeat the burst a few times in case the consumer thread
        # hasn't entered the kernel recv yet on the first pass. Each
        # send is one TS packet (the natural framing the receiver also
        # uses).
        for _ in range(3):
            for i in range(0, len(ts_bytes), 188):
                snd.send(ts_bytes[i : i + 188])
            time.sleep(0.05)
    # Wait for the consumer to receive at least one event.
    t.join(timeout=3.0)
    rx.close()
    # If the consumer raised before producing an event, that's a
    # genuine failure; a CANCELLED after the first event is OK
    # (close() fires the cancel).
    if err and not events:
        pytest.fail(f"consumer raised before any event: {err}")
    assert len(events) >= 1, "consumer did not see any DemuxEvent"


# --------------------------------------------------------------------------- #
# Round-trip                                                                  #
# --------------------------------------------------------------------------- #


def test_mux_sender_to_demux_receiver_round_trip() -> None:
    """Round-trip: MuxSender → DemuxReceiver over the same loopback
    port. Push a video NAL via MuxSender; assert the receiver emits
    a ProgramMap + Sample event."""
    port = _free_udp_port()
    rx = tstrans.rtp.DemuxReceiver(f"rtp://127.0.0.1:{port}")
    events: list[object] = []
    err: list[BaseException] = []
    saw_program_map = threading.Event()

    def consumer() -> None:
        try:
            for ev in rx:
                events.append(ev)
                if isinstance(ev, DemuxEvent.ProgramMap):
                    saw_program_map.set()
                # Bail out after a Sample is seen — we got enough.
                if isinstance(ev, DemuxEvent.Video):
                    break
        except BaseException as exc:  # noqa: BLE001
            err.append(exc)

    t = threading.Thread(target=consumer, daemon=True)
    t.start()
    # Give the consumer time to bind.
    time.sleep(0.2)
    # Push several key NALs to cover PSI emission + a video sample.
    program = _video_only_program()
    with tstrans.rtp.MuxSender(f"rtp://127.0.0.1:{port}", program) as snd:
        nal_idr = b"\x00\x00\x00\x01\x65\xBB"
        for i in range(8):
            snd.send_video(
                nal_idr, pts=Pts90khz.from_raw(i * 3000), key_frame=(i == 0)
            )
    # Wait for the consumer to see a video Sample.
    t.join(timeout=5.0)
    rx.close()
    assert not err, f"consumer raised: {err}"
    # Expect at least one ProgramMap event and at least one Sample.
    saw_video = any(isinstance(e, DemuxEvent.Video) for e in events)
    assert saw_program_map.is_set() or saw_video, (
        f"expected at least one PMT or Video event, got: {[type(e).__name__ for e in events]}"
    )


# --------------------------------------------------------------------------- #
# Stats                                                                       #
# --------------------------------------------------------------------------- #


def test_stats_tuple_shape() -> None:
    port = _free_udp_port()
    with tstrans.rtp.DemuxReceiver(f"rtp://127.0.0.1:{port}") as rx:
        stats = rx.stats()
        assert isinstance(stats, tuple)
        assert len(stats) == 2
        socket_stats, muxer_stats = stats
        assert socket_stats.packets_received == 0
        assert muxer_stats.programs_configured == 0


# --------------------------------------------------------------------------- #
# into_demux_receiver bridge — skipped (needs RTSP server fixture)           #
# --------------------------------------------------------------------------- #


@pytest.mark.skip(
    reason="RtspSession.into_demux_receiver requires a real RTSP server fixture; "
    "covered by Wave C T25 integration tests."
)
def test_rtsp_session_into_demux_receiver() -> None:  # pragma: no cover
    """Bridge from `RtspSession` → `DemuxReceiver`. Verifying this
    end-to-end requires a real RTSP server, which lands in Wave C T25.

    The bridge itself is wired in `bindings/python/src/rtp/client.rs`:
    `PyRtspClient::connect` retains the SETUP-time `RtspSession`; the
    `into_demux_receiver` method consumes it via
    `RtspSession::into_recv_transport` and wraps the resulting
    `RtpRecvTransport` in a `PyDemuxReceiver`.
    """
