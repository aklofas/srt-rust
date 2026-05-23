"""ST 0102 + ST 0605 encode round-trip tests."""

from tstrans.klv import (
    ClassifyingCountryCodingMethod,
    ObjectCountryCodingMethod,
    PrecisionTimeStampPack,
    SecurityClassification,
    SecurityLs,
    TimeStatus,
    decode_precision_timestamp,
    decode_security,
    encode_precision_timestamp,
    encode_security,
)


# ---------------------------------------------------------------------------
# ST 0102 — encode_security
# ---------------------------------------------------------------------------


def test_encode_security_round_trip_basic():
    # Minimal ST 0102 body: Tag 1 (Security Classification) = 1 byte (0x01 =
    # UNCLASSIFIED). Lenient decode tolerates missing required tags.
    body = b"\x01\x01\x01"
    rec = decode_security(body)
    out = encode_security(rec)
    assert isinstance(out, bytes)
    rec2 = decode_security(out)
    assert rec2.security_classification == rec.security_classification
    assert rec2.security_classification == SecurityClassification.UNCLASSIFIED


def test_encode_security_returns_bytes_not_bytearray():
    rec = decode_security(b"\x01\x01\x01")
    out = encode_security(rec)
    assert isinstance(out, bytes)


def test_encode_security_empty_record_returns_empty_body():
    # All-None record encodes to a zero-length body.
    rec = SecurityLs()
    out = encode_security(rec)
    assert isinstance(out, bytes)
    assert out == b""


def test_encode_security_full_round_trip_preserves_typed_enums_and_strings():
    # Hand-build a record with all 3 typed-enum fields + a UTF-16 string field.
    rec = SecurityLs(
        security_classification=SecurityClassification.SECRET,
        classifying_country_coding_method=ClassifyingCountryCodingMethod.ISO_3166_TWO_LETTER,
        classifying_country="//US",
        object_country_coding_method=ObjectCountryCodingMethod.ISO_3166_NUMERIC,
        object_country_codes="US;CA",
        version=12,
        caveats="NOFORN",
        classified_by="OCA: Some Body",
    )
    out = encode_security(rec)
    assert isinstance(out, bytes)
    assert len(out) > 0
    rec2 = decode_security(out)
    assert rec2.security_classification == SecurityClassification.SECRET
    assert (
        rec2.classifying_country_coding_method
        == ClassifyingCountryCodingMethod.ISO_3166_TWO_LETTER
    )
    assert rec2.classifying_country == "//US"
    assert rec2.object_country_coding_method == ObjectCountryCodingMethod.ISO_3166_NUMERIC
    assert rec2.object_country_codes == "US;CA"
    assert rec2.version == 12
    assert rec2.caveats == "NOFORN"
    assert rec2.classified_by == "OCA: Some Body"


# ---------------------------------------------------------------------------
# ST 0605 — encode_precision_timestamp
# ---------------------------------------------------------------------------


def test_encode_precision_timestamp_is_26_bytes():
    pack = PrecisionTimeStampPack(time_status=TimeStatus(raw=0x1F), timestamp_us=0)
    out = encode_precision_timestamp(pack)
    assert isinstance(out, bytes)
    assert len(out) == 26


def test_encode_precision_timestamp_round_trip_preserves_fields():
    pack = PrecisionTimeStampPack(
        time_status=TimeStatus(raw=0x1F),
        timestamp_us=1_700_000_000_000_000,
    )
    out = encode_precision_timestamp(pack)
    pack2 = decode_precision_timestamp(out)
    assert pack2.timestamp_us == pack.timestamp_us
    assert pack2.time_status.raw == pack.time_status.raw
