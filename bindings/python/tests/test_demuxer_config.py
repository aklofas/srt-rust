"""DemuxerConfig — Phase 2 minimal config (strict mode + PES caps).
Advanced knobs (link_klv, treat_as, av1_carriage) deferred."""

from tstrans.mpegts import DemuxerConfig, StrictMode


def test_default_construction():
    cfg = DemuxerConfig()
    assert cfg.strict_mode is StrictMode.OFF
    # Defaults track the Rust DemuxerConfig defaults (current values:
    # 4 MB per-PID, 64 MB total — verified in
    # tst-core/src/mpegts/demux/types.rs DEFAULT_PES_CAP_* constants).
    assert cfg.pes_cap_per_pid == 4 * 1024 * 1024
    assert cfg.pes_cap_total == 64 * 1024 * 1024


def test_overrides_via_kwargs():
    cfg = DemuxerConfig(
        strict_mode=StrictMode.FULL,
        pes_cap_per_pid=1024,
        pes_cap_total=8192,
    )
    assert cfg.strict_mode is StrictMode.FULL
    assert cfg.pes_cap_per_pid == 1024
    assert cfg.pes_cap_total == 8192


def test_immutable():
    import dataclasses
    cfg = DemuxerConfig()
    try:
        cfg.strict_mode = StrictMode.FULL
    except dataclasses.FrozenInstanceError:
        pass
    else:
        raise AssertionError("DemuxerConfig should be frozen")
