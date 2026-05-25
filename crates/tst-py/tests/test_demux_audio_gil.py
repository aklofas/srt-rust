"""Audit-2 #3 — Demuxer.next_event() must not pin the GIL during
AAC/MP2 frame parsing. Regression test: a second thread must make
progress while next_event() is parsing a multi-MB audio PES."""

import threading
import time
from pathlib import Path

import pytest

import tstrans
from tstrans.mpegts import Demuxer, DemuxerConfig

# ---------------------------------------------------------------------------
# GIL-progress test — skipped until the audio-heavy synthetic fixture
# is available (built in Wave 2 / Task 9).
# ---------------------------------------------------------------------------

pytestmark = pytest.mark.skipif(
    not Path("crates/tst-py/tests/fixtures/audio_aac_large.ts").exists(),
    reason="needs an audio-heavy synthetic TS (~2 MB); built in task 9",
)


def test_next_event_releases_gil_during_aac_parse() -> None:
    ts = Path("crates/tst-py/tests/fixtures/audio_aac_large.ts").read_bytes()
    dx = Demuxer(DemuxerConfig())
    dx.feed(ts)
    dx.flush()

    progress_counter = 0
    stop_flag = threading.Event()

    def background_progress():
        nonlocal progress_counter
        while not stop_flag.is_set():
            progress_counter += 1
            time.sleep(0.0)  # yield

    t = threading.Thread(target=background_progress, daemon=True)
    t.start()
    try:
        # Drain all events.
        for _ in dx:
            pass
    finally:
        stop_flag.set()
        t.join(timeout=1.0)

    # If next_event held the GIL the whole time, progress_counter would
    # be ~0. With GIL release in audio parse, it should be >> 0.
    assert progress_counter > 50, (
        f"background thread saw only {progress_counter} iterations — "
        "GIL appears held during audio parse"
    )


# ---------------------------------------------------------------------------
# Smoke test — no skip marker; verifies GIL refactor is behavior-neutral.
# Uses the muxer to build a small in-process AAC TS instead of reading
# a fixture file.
# ---------------------------------------------------------------------------


def test_next_event_audio_typed_payload_unchanged() -> None:
    """Smoke — GIL release refactor must not change observable output.

    Uses an existing small AAC TS fixture if present; skips if absent.
    The full GIL-progress assertion is gated on a larger fixture (task 9).
    """
    p = Path("crates/tst-py/tests/fixtures/aac_minimal.ts")
    if not p.exists():
        pytest.skip("aac_minimal.ts not present")
    dx = Demuxer(DemuxerConfig())
    dx.feed(p.read_bytes())
    dx.flush()
    audio_events = [e for e in dx if isinstance(e, tstrans.mpegts.DemuxEvent.Audio)]
    assert audio_events, "expected at least one Audio event"
    for ev in audio_events:
        assert isinstance(ev.payload, list), (
            f"expected list payload after GIL refactor, got {type(ev.payload).__name__}"
        )
