"""F10 — DemuxerConfig.__post_init__ fail-fast validation.

Mirrors the Audit-2 #4 pattern applied to `Pts90khz` and the KLV
dataclasses: invalid primitive shapes (wrong type, out-of-range) must
fail at construction, not deep inside `build_demuxer` Rust extraction.
"""

import pytest

from tstrans.mpegts import DemuxerConfig, StrictMode


def test_valid_default_construction() -> None:
    cfg = DemuxerConfig()
    assert cfg.strict_mode is StrictMode.OFF


def test_valid_explicit_construction() -> None:
    cfg = DemuxerConfig(
        strict_mode=StrictMode.FULL,
        pes_cap_per_pid=1024,
        pes_cap_total=8192,
        cfi_tolerance=True,
    )
    assert cfg.strict_mode is StrictMode.FULL
    assert cfg.pes_cap_per_pid == 1024
    assert cfg.pes_cap_total == 8192
    assert cfg.cfi_tolerance is True


def test_strict_mode_string_rejected_with_typeerror() -> None:
    with pytest.raises(TypeError, match="strict_mode"):
        DemuxerConfig(strict_mode="strict")  # type: ignore[arg-type]


def test_strict_mode_int_rejected_with_typeerror() -> None:
    with pytest.raises(TypeError, match="strict_mode"):
        DemuxerConfig(strict_mode=0)  # type: ignore[arg-type]


def test_pes_cap_per_pid_negative_rejected_with_valueerror() -> None:
    with pytest.raises(ValueError, match="pes_cap_per_pid"):
        DemuxerConfig(pes_cap_per_pid=-1)


def test_pes_cap_per_pid_zero_rejected_with_valueerror() -> None:
    # 0 is technically a valid usize on the Rust side, but it makes
    # the demuxer unusable (no PES can ever reassemble). Reject loudly.
    with pytest.raises(ValueError, match="pes_cap_per_pid"):
        DemuxerConfig(pes_cap_per_pid=0)


def test_pes_cap_per_pid_string_rejected_with_typeerror() -> None:
    with pytest.raises(TypeError, match="pes_cap_per_pid"):
        DemuxerConfig(pes_cap_per_pid="4194304")  # type: ignore[arg-type]


def test_pes_cap_per_pid_bool_rejected_with_typeerror() -> None:
    # `bool` is an `int` subclass in Python; the check must exclude it
    # explicitly or `DemuxerConfig(pes_cap_per_pid=True)` (== 1 byte!)
    # silently passes and produces a near-useless demuxer.
    with pytest.raises(TypeError, match="pes_cap_per_pid"):
        DemuxerConfig(pes_cap_per_pid=True)  # type: ignore[arg-type]


def test_pes_cap_total_negative_rejected_with_valueerror() -> None:
    with pytest.raises(ValueError, match="pes_cap_total"):
        DemuxerConfig(pes_cap_total=-1)


def test_pes_cap_total_zero_rejected_with_valueerror() -> None:
    with pytest.raises(ValueError, match="pes_cap_total"):
        DemuxerConfig(pes_cap_total=0)


def test_pes_cap_total_bool_rejected_with_typeerror() -> None:
    with pytest.raises(TypeError, match="pes_cap_total"):
        DemuxerConfig(pes_cap_total=True)  # type: ignore[arg-type]


def test_cfi_tolerance_non_bool_rejected_with_typeerror() -> None:
    with pytest.raises(TypeError, match="cfi_tolerance"):
        DemuxerConfig(cfi_tolerance=1)  # type: ignore[arg-type]
    with pytest.raises(TypeError, match="cfi_tolerance"):
        DemuxerConfig(cfi_tolerance="false")  # type: ignore[arg-type]
