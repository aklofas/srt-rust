"""Tests for `tstrans.rtp.H264Receiver` and `RtspClient.connect_h264` (Task 14).

Exercises the RFC 6184 H.264 receiver Python surface:

1. Single-NALU UDP loopback — bind H264Receiver on an ephemeral port,
   send a hand-built IDR packet matching the Rust test's byte layout,
   assert annexb / pts / key_frame.
2. Iterator protocol — send 2 AUs then close from a timer thread;
   collect via list(receiver).
3. Close-then-recv raises an exception consistent with DemuxReceiver's
   closed-handle error (RtpError family).
4. Config kwargs round-trip — H264DepayConfig construction + defaults.
5. Missing ?pt= URL errors surface as RtpError.
"""

from __future__ import annotations

import socket
import threading
import time

import pytest

import tstrans.rtp
from tstrans.exceptions import RtpError, RtpErrorKind


# --------------------------------------------------------------------------- #
# Helpers                                                                     #
# --------------------------------------------------------------------------- #


def _rtp_pkt(seq: int, ts: int, pt: int = 96, nal_bytes: bytes = b"\x65\xAB\xCD") -> bytes:
    """Build a minimal single-NALU RTP packet (RFC 3550 §5.1).

    Byte layout (12-byte fixed header, no CSRC, no extension):
      0:  0x80  (V=2, P=0, X=0, CC=0)
      1:  0x80 | pt  (M=1, PT=pt)
      2..4: seq (big-endian u16)
      4..8: ts  (big-endian u32)
      8..12: ssrc=9 (big-endian u32)
      12..: nal_bytes
    """
    header = bytes([
        0x80,
        0x80 | (pt & 0x7F),
        (seq >> 8) & 0xFF, seq & 0xFF,
        (ts >> 24) & 0xFF, (ts >> 16) & 0xFF, (ts >> 8) & 0xFF, ts & 0xFF,
        0, 0, 0, 9,  # ssrc = 9
    ])
    return header + nal_bytes


# --------------------------------------------------------------------------- #
# 1. Single-NALU UDP loopback                                                  #
# --------------------------------------------------------------------------- #


def test_h264_receiver_single_au_loopback() -> None:
    """Bind H264Receiver on an ephemeral port, send one IDR packet, assert AU.

    Mirrors the Rust unit test in crates/tst-rtp/src/h264/receiver.rs:
      pkt bytes: 0x80, 0x80|96, 0, 1, 0, 0, 0x23, 0x28, 0, 0, 0, 9, 0x65, 0xAB, 0xCD
      rtp_timestamp = 0x00002328 = 9000
      Expected annexb = b"\\x00\\x00\\x00\\x01\\x65\\xab\\xcd"
      key_frame = True (NALU type 5 = IDR)
    """
    rx = tstrans.rtp.H264Receiver.listen("rtp://127.0.0.1:0?pt=96")
    addr_str = rx.local_addr()
    assert addr_str is not None, "local_addr() must not be None for UDP"
    host, _, port_str = addr_str.rpartition(":")
    port = int(port_str)

    # Hand-built packet matching the Rust test exactly.
    pkt = bytes([
        0x80, 0x80 | 96,
        0, 1,          # seq = 1
        0, 0, 0x23, 0x28,  # ts = 9000
        0, 0, 0, 9,    # ssrc = 9
        0x65, 0xAB, 0xCD,  # IDR NALU
    ])
    tx = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    tx.sendto(pkt, (host, port))
    tx.close()

    au = rx.recv_au()
    assert au is not None, "recv_au() must return an AU after one packet"
    assert bytes(au.annexb) == b"\x00\x00\x00\x01\x65\xab\xcd"
    assert au.key_frame is True
    # pts is 90kHz int; ts=9000 = 0x2328 maps through depacketizer
    assert isinstance(au.pts, int)
    assert au.rtp_timestamp == 9000

    rx.close()


# --------------------------------------------------------------------------- #
# 2. Iterator protocol                                                          #
# --------------------------------------------------------------------------- #


def test_h264_receiver_iterator_collects_aus() -> None:
    """Send 2 IDR AUs via loopback, then close; list(rx) collects them."""
    rx = tstrans.rtp.H264Receiver.listen("rtp://127.0.0.1:0?pt=96")
    addr_str = rx.local_addr()
    assert addr_str is not None
    host, _, port_str = addr_str.rpartition(":")
    port = int(port_str)

    collected: list[object] = []
    done = threading.Event()

    def consumer() -> None:
        # Collect up to 2 AUs then stop
        for au in rx:
            collected.append(au)
            if len(collected) >= 2:
                break
        done.set()

    t = threading.Thread(target=consumer, daemon=True)
    t.start()

    time.sleep(0.05)  # let consumer park on recv_au

    tx = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    # Send two distinct packets (different timestamps so depacketizer emits 2 AUs)
    pkt1 = _rtp_pkt(seq=1, ts=0, nal_bytes=b"\x65\x01")
    pkt2 = _rtp_pkt(seq=2, ts=3000, nal_bytes=b"\x65\x02")
    tx.sendto(pkt1, (host, port))
    time.sleep(0.01)
    tx.sendto(pkt2, (host, port))
    tx.close()

    done.wait(timeout=3.0)
    rx.close()
    t.join(timeout=1.0)

    assert len(collected) >= 1, (
        f"expected at least 1 AU from iterator, got {len(collected)}"
    )
    for au in collected:
        assert bytes(au.annexb).startswith(b"\x00\x00\x00\x01")


# --------------------------------------------------------------------------- #
# 3. Close-then-recv raises RtpError                                           #
# --------------------------------------------------------------------------- #


def test_h264_receiver_closed_recv_raises() -> None:
    """Closing then calling recv_au must raise RtpError (TRANSPORT or CANCELLED)."""
    rx = tstrans.rtp.H264Receiver.listen("rtp://127.0.0.1:0?pt=96")
    rx.close()
    with pytest.raises(RtpError) as exc_info:
        rx.recv_au()
    # Either TRANSPORT or CANCELLED is acceptable for a closed receiver.
    assert exc_info.value.kind in (RtpErrorKind.TRANSPORT, RtpErrorKind.CANCELLED)


# --------------------------------------------------------------------------- #
# 4. Config kwargs round-trip                                                   #
# --------------------------------------------------------------------------- #


def test_h264_depay_config_defaults() -> None:
    """H264DepayConfig() with no args must reflect Rust defaults."""
    cfg = tstrans.rtp.H264DepayConfig()
    assert cfg.payload_type == 96
    assert cfg.parameter_set_injection == tstrans.rtp.ParameterSetInjection.BEFORE_IDR
    assert list(cfg.initial_parameter_sets) == []
    assert cfg.max_au_bytes == 8 * 1024 * 1024


def test_h264_depay_config_kwargs() -> None:
    """H264DepayConfig with explicit kwargs round-trips via getters."""
    cfg = tstrans.rtp.H264DepayConfig(
        payload_type=100,
        parameter_set_injection=tstrans.rtp.ParameterSetInjection.NONE,
        initial_parameter_sets=[b"\x67\x01\x02", b"\x68\x03"],
        max_au_bytes=1024 * 1024,
    )
    assert cfg.payload_type == 100
    assert cfg.parameter_set_injection == tstrans.rtp.ParameterSetInjection.NONE
    assert list(cfg.initial_parameter_sets) == [b"\x67\x01\x02", b"\x68\x03"]
    assert cfg.max_au_bytes == 1024 * 1024


def test_parameter_set_injection_enum() -> None:
    """ParameterSetInjection has NONE and BEFORE_IDR variants."""
    assert tstrans.rtp.ParameterSetInjection.NONE != tstrans.rtp.ParameterSetInjection.BEFORE_IDR
    assert isinstance(tstrans.rtp.ParameterSetInjection.NONE, tstrans.rtp.ParameterSetInjection)


# --------------------------------------------------------------------------- #
# 5. Missing ?pt= raises RtpError                                               #
# --------------------------------------------------------------------------- #


def test_h264_receiver_listen_without_pt_raises() -> None:
    """listen() without ?pt= must raise RtpError(TRANSPORT)."""
    with pytest.raises(RtpError) as exc_info:
        tstrans.rtp.H264Receiver.listen("rtp://127.0.0.1:0")
    assert exc_info.value.kind == RtpErrorKind.TRANSPORT


# --------------------------------------------------------------------------- #
# 6. Module surface                                                             #
# --------------------------------------------------------------------------- #


def test_h264_receiver_module_re_exports() -> None:
    """All new public names visible in tstrans.rtp."""
    assert tstrans.rtp.H264Receiver is not None
    assert tstrans.rtp.H264AccessUnit is not None
    assert tstrans.rtp.H264DepayConfig is not None
    assert tstrans.rtp.H264DepayStats is not None
    assert tstrans.rtp.ParameterSetInjection is not None
    assert "H264Receiver" in tstrans.rtp.__all__
    assert "H264DepayConfig" in tstrans.rtp.__all__
    assert "H264DepayStats" in tstrans.rtp.__all__
    assert "ParameterSetInjection" in tstrans.rtp.__all__


def test_h264_receiver_stats_shape() -> None:
    """H264Receiver exposes depay_stats(), rtp_stats(), socket_stats() without error."""
    rx = tstrans.rtp.H264Receiver.listen("rtp://127.0.0.1:0?pt=96")
    ds = rx.depay_stats()
    assert isinstance(ds, tstrans.rtp.H264DepayStats)
    assert ds.aus_emitted == 0
    assert ds.aus_dropped == 0

    rs = rx.rtp_stats()
    # RtpStats has malformed_packets
    assert hasattr(rs, "malformed_packets")
    assert rs.malformed_packets == 0

    ss = rx.socket_stats()
    assert isinstance(ss, tstrans.rtp.SocketStats)
    rx.close()
