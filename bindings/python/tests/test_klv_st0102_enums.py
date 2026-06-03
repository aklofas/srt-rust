"""ST 0102.12 §6.1 enums — codepoints differ between Tags 2 and 12."""

import pytest

from tstrans.klv import (
    ClassifyingCountryCodingMethod,
    ObjectCountryCodingMethod,
    SecurityClassification,
)


def test_security_classification_codepoints():
    assert SecurityClassification.UNCLASSIFIED.value == 0x01
    assert SecurityClassification.RESTRICTED.value == 0x02
    assert SecurityClassification.CONFIDENTIAL.value == 0x03
    assert SecurityClassification.SECRET.value == 0x04
    assert SecurityClassification.TOP_SECRET.value == 0x05


def test_security_classification_all_variants():
    expected = {"UNCLASSIFIED", "RESTRICTED", "CONFIDENTIAL", "SECRET", "TOP_SECRET"}
    assert {v.name for v in SecurityClassification} == expected


def test_classifying_country_codepoints():
    assert ClassifyingCountryCodingMethod.ISO_3166_TWO_LETTER.value == 0x01
    assert ClassifyingCountryCodingMethod.ISO_3166_THREE_LETTER.value == 0x02
    assert ClassifyingCountryCodingMethod.FIPS_104_TWO_LETTER.value == 0x03
    assert ClassifyingCountryCodingMethod.FIPS_104_FOUR_LETTER.value == 0x04
    assert ClassifyingCountryCodingMethod.ISO_3166_NUMERIC.value == 0x05
    assert ClassifyingCountryCodingMethod.GENC_MIXED.value == 0x10
    assert ClassifyingCountryCodingMethod.OMITTED_VALUE_08.value == 0x08
    assert ClassifyingCountryCodingMethod.OMITTED_VALUE_09.value == 0x09


def test_object_country_codepoints():
    assert ObjectCountryCodingMethod.ISO_3166_TWO_LETTER.value == 0x01
    assert ObjectCountryCodingMethod.ISO_3166_THREE_LETTER.value == 0x02
    assert ObjectCountryCodingMethod.ISO_3166_NUMERIC.value == 0x03  # vs Tag 2's 0x05
    assert ObjectCountryCodingMethod.FIPS_104_TWO_LETTER.value == 0x04
    assert ObjectCountryCodingMethod.FIPS_104_FOUR_LETTER.value == 0x05
    assert ObjectCountryCodingMethod.GENC_ADMIN_SUB.value == 0x40


def test_iso3166_numeric_codepoint_differs_between_tags():
    assert (
        ClassifyingCountryCodingMethod.ISO_3166_NUMERIC.value
        != ObjectCountryCodingMethod.ISO_3166_NUMERIC.value
    )
    assert ClassifyingCountryCodingMethod.ISO_3166_NUMERIC.value == 0x05
    assert ObjectCountryCodingMethod.ISO_3166_NUMERIC.value == 0x03
