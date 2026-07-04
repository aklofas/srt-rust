"""mypy --strict assert_type smoke for tstrans.io — pins the load-bearing
io signatures against the .pyi stub. Not collected by pytest (no test_*);
checked statically by tests/typing/mypy.ini."""
from typing import Iterator, assert_type

from tstrans.io import (
    ProbeResult,
    Transmuxer,
    extract_klv,
    iter_uas_datalink,
    parse_file,
    probe,
    transmux,
)
from tstrans.klv import UasDatalinkLs
from tstrans.mpegts import DemuxEvent, Pts90khz, StreamKindTag

assert_type(parse_file("a.ts"), Iterator[DemuxEvent])
r = probe("a.ts")
assert_type(r, ProbeResult)
assert_type(r.size_bytes, int)
assert_type(r.has_klv, bool)
# extract_klv is multi-mode; stub returns Iterator[Any] (matches runtime's
# bare `Iterator`), so just confirm it type-checks as an iterator:
for _ in extract_klv("a.ts", with_pts=True):
    pass

# iter_uas_datalink yields (pts, klv_index, record) with precise types.
for pts, klv_index, record in iter_uas_datalink("a.ts", strict=True):
    assert_type(pts, Pts90khz)
    assert_type(klv_index, int)
    assert_type(record, UasDatalinkLs)

# transmux: context manager + iterator + write surface, precise types.
with transmux("a.ts", "b.ts", drop=(StreamKindTag.KLV_SYNC,), atomic=True) as tx:
    assert_type(tx, Transmuxer)
    for ev in tx:
        assert_type(ev, DemuxEvent)
        tx.write(ev)
        if isinstance(ev, DemuxEvent.Metadata):
            tx.write_klv(ev, b"replacement")
