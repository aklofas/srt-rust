"""mypy --strict assert_type smoke for tstrans.klv — pins the decode/encode +
parse_klv_universal surface used by the KLV-corrector workflow.
Not collected by pytest (no test_*); checked statically by tests/typing/mypy.ini.

Note: smoke files are static-checked only, never executed — the byte literals
just need to be valid `bytes`, they are never decoded at runtime."""
from typing import Optional, Union, assert_type

from tstrans.klv import (
    GeoPoint,
    PrecisionTimeStampPack,
    SecurityLs,
    UasDatalinkLs,
    VmtiLs,
    decode_security,
    decode_uas_datalink,
    decode_vmti,
    encode_security,
    encode_uas_datalink,
    is_st0601_family,
    parse_klv_universal,
)

# UAS Datalink decode → encode round-trip.
ls = decode_uas_datalink(b"\x06\x0e")
assert_type(ls, UasDatalinkLs)
raw = encode_uas_datalink(ls)
assert_type(raw, bytes)

# Composite-view accessor + an Optional field on the decoded record.
assert_type(ls.sensor_position(), Optional[GeoPoint])
assert_type(ls.timestamp_us, Optional[int])

# Security LS decode → encode round-trip + an Optional field.
sec = decode_security(b"\x01\x01\x01")
assert_type(sec, SecurityLs)
assert_type(encode_security(sec), bytes)
assert_type(sec.classifying_country, Optional[str])

# VMTI LS decode.
vmti = decode_vmti(b"\x03\x02\x00\x06")
assert_type(vmti, VmtiLs)

assert_type(is_st0601_family(b"\x06\x0e"), bool)

# Pin the universal dispatcher's Optional[union] return — guards against the
# union being accidentally widened/narrowed.
assert_type(
    parse_klv_universal(b"\x06\x0e"),
    Optional[Union[UasDatalinkLs, SecurityLs, PrecisionTimeStampPack, VmtiLs]],
)
