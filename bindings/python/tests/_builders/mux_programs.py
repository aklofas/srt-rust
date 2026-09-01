"""Minimal `MuxerProgramConfig` builders shared by mux/demux tests.

These build just enough program shape to exercise a single stream kind;
tests that need additional streams (KLV, data, audio) layer their own
`.add_*` calls on top of `MuxerProgramConfigBuilder` directly rather than
extending these helpers.
"""

from __future__ import annotations

from tstrans.mpegts import MuxerProgramConfigBuilder, VideoCodec


def video_only_program(pid_video: int = 0x101) -> object:
    """Minimal single-video-stream `MuxerProgramConfig` (program 1, PMT
    PID 0x100)."""
    return (
        MuxerProgramConfigBuilder(1, 0x100)
        .add_video(pid_video, VideoCodec.H264)
        .build()
    )
