"""mypy --strict assert_type smoke for tstrans.mpegts — pins the raw-first
DemuxEvent.Video/.Audio surface (.raw + .parse(), NOT .payload), Pts90khz, and
a few representative mux-side members so accidental signature drift fails mypy
here too (not just under stubtest).
Not collected by pytest (no test_*); checked statically by tests/typing/mypy.ini.

Note: smoke files are static-checked only, never executed — the byte literals
just need to be valid `bytes`, they are never muxed/demuxed at runtime."""
from typing import Any, List, Literal, Optional, Tuple, Union, assert_type

from tstrans.mpegts import (
    DemuxEvent,
    Demuxer,
    MuxerConfigBuilder,
    Pts90khz,
    VideoCodec,
)

v: DemuxEvent.Video
assert_type(v.raw, bytes)
assert_type(v.random_access_indicator, bool)
assert_type(v.pts, Pts90khz)
assert_type(v.dts, Optional[Pts90khz])
assert_type(v.codec, VideoCodec)
# parse() is mode-polymorphic: strict=False → bare unit list; strict=True →
# (units, issues) tuple. The stub declares the union of both, so just pin the
# declared return type rather than a single concrete shape.
parsed = v.parse(strict=False)
assert_type(parsed, Union[List[Any], Tuple[List[Any], List[str]]])

a: DemuxEvent.Audio
assert_type(a.raw, bytes)
a.parse(strict=False)

# Pts90khz surface — a constructor and both derived props.
p = Pts90khz.from_raw(9000)
assert_type(p, Pts90khz)
assert_type(p.ms, int)
assert_type(p.seconds, float)
assert_type(Pts90khz.from_ms(100), Pts90khz)

# Representative mux/demux coverage beyond the raw-first pair — a Demuxer
# method return, a builder-chaining call returning the builder self-type, and
# an enum-member access. These pin the symbols so accidental signature drift
# would fail mypy here. (VideoCodec is a real enum.Enum, so a member access
# narrows to its Literal type.)
d: Demuxer
assert_type(d.next_event(), Optional[DemuxEvent])
b = MuxerConfigBuilder()
assert_type(b.buffer_packets(1024), MuxerConfigBuilder)
assert_type(VideoCodec.H264, Literal[VideoCodec.H264])
