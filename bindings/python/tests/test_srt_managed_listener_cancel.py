"""cancel() must wake a listener-mode ManagedDemuxReceiver whose reconnect is
parked in re-accept after its peer disconnected.

Python mirror of tst-c's `loopback_cancel_wakes_managed_listener_parked_in_reaccept`
(ROADMAP "cancellable managed-listener re-accept"). Before the fix the
listener factory sat in `Listener::accept()` with nothing able to reach that
listener, and the backoff between attempts was an uninterruptible sleep, so
`cancel()` did nothing until the next peer happened to connect.

Choreography:
  1. Accept thread: `ManagedDemuxReceiver.from_url("srt://:P?mode=listener")`
     (blocks until a peer connects).
  2. Main: a `ManagedMuxSender` caller connects, pushes a few frames, then
     closes — the managed receiver sees the break and re-enters its factory
     (bind + accept) after the first backoff.
  3. Main: `cancel_handle().cancel()`; the iterator thread must end within a
     couple of seconds with `SrtError(CLOSED)` (the Python surface maps a
     caller-initiated close to CLOSED; see `errors.rs`).

If the cancel does NOT wake the accept, a rescue peer is connected so the
daemon thread can be joined and the test fails with a clear message rather
than leaving a thread parked in native accept at interpreter exit.
"""

from __future__ import annotations

import threading
import time

import pytest

import tstrans.srt as srt
from tstrans.exceptions import SrtError, SrtErrorKind
from tstrans.mpegts import Pts90khz

from _builders.mux_programs import video_only_program as _video_only_program
from _builders.ports import free_tcp_port as _free_tcp_port

NAL_IDR = b"\x00\x00\x00\x01\x65\xBB"


def _connect_sender(caller_url: str, budget_s: float) -> srt.ManagedMuxSender:
    """Connect a caller, retrying while the listener is between binds."""
    deadline = time.monotonic() + budget_s
    last: BaseException | None = None
    while time.monotonic() < deadline:
        try:
            return srt.ManagedMuxSender.from_url(caller_url, _video_only_program())
        except SrtError as exc:  # listener not (re)bound yet
            last = exc
            time.sleep(0.05)
    pytest.fail(f"caller could not connect within {budget_s}s: {last}")


def test_cancel_wakes_managed_listener_parked_in_reaccept() -> None:
    port = _free_tcp_port()
    listener_url = f"srt://:{port}?mode=listener"
    caller_url = f"srt://127.0.0.1:{port}?mode=caller"

    rx_box: list[srt.ManagedDemuxReceiver] = []
    rx_err: list[BaseException] = []

    def accept_worker() -> None:
        try:
            rx_box.append(srt.ManagedDemuxReceiver.from_url(listener_url))
        except BaseException as exc:  # noqa: BLE001
            rx_err.append(exc)

    accept_t = threading.Thread(target=accept_worker, daemon=True)
    accept_t.start()
    time.sleep(0.1)

    sender = _connect_sender(caller_url, 5.0)
    accept_t.join(timeout=5.0)
    if rx_err:
        sender.close()
        pytest.fail(f"ManagedDemuxReceiver accept failed: {rx_err[0]}")
    if not rx_box:
        sender.close()
        pytest.fail("ManagedDemuxReceiver listener thread did not accept within 5 s")
    rx = rx_box[0]

    # A few frames so the link is genuinely up before the peer drops.
    for i in range(5):
        sender.send_video(NAL_IDR, pts=Pts90khz.from_raw(i * 3000), key_frame=(i == 0))
        time.sleep(0.01)

    outcome: dict[str, object] = {}

    def iterator() -> None:
        try:
            for _ev in rx:
                pass
            outcome["end"] = "StopIteration"
        except SrtError as exc:
            outcome["end"] = "SrtError"
            outcome["kind"] = exc.kind
        except Exception as exc:  # noqa: BLE001
            outcome["end"] = type(exc).__name__

    iter_t = threading.Thread(target=iterator, daemon=True)
    iter_t.start()
    time.sleep(0.3)

    # Peer drop: the managed receiver re-enters its factory (bind + accept)
    # after the default 100 ms backoff and parks there with no peer in sight.
    sender.close()
    time.sleep(1.0)

    t0 = time.monotonic()
    rx.cancel_handle().cancel()
    iter_t.join(timeout=3.0)
    woke_after = time.monotonic() - t0

    if iter_t.is_alive():
        # Rescue so the daemon thread is not left parked in native accept.
        rescue = _connect_sender(caller_url, 5.0)
        iter_t.join(timeout=5.0)
        rescue.close()
        pytest.fail("cancel() did not wake the managed listener parked in re-accept within 3 s")

    assert woke_after < 2.0, f"cancel took {woke_after:.2f}s to wake the parked re-accept"
    assert outcome.get("end") == "SrtError", f"iteration ended via {outcome}"
    assert outcome.get("kind") == SrtErrorKind.CLOSED, f"unexpected SrtError kind: {outcome}"
    rx.close()
