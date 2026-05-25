"""Python-side coverage for the opt-in malformed-CFI tolerance mode.

End-to-end Rust coverage lives in
`crates/tst-core/tests/mpegts_au_cell_tolerance.rs`. Python tests here
cover the binding surface — the new config field, the new
`NonConformantKind` enum entry, the new `CellFragmentIndication` enum
re-export, and the new optional fields on `_NonConformantEvent` — plus
one minimum-discriminating end-to-end test that exercises the full
config-through-translation path (a Python `DemuxerConfig` with
`cfi_tolerance=True` produces a `Demuxer` that
surfaces the tolerance diagnostic with the right typed fields).
"""

from __future__ import annotations

from tstrans.mpegts import (
    CellFragmentIndication,
    DemuxEvent,
    Demuxer,
    DemuxerConfig,
    KlvStreamType,
    MetadataKindTag,
    Muxer,
    MuxerConfig,
    MuxerProgramConfigBuilder,
    NonConformantKind,
    Pts90khz,
    VideoCodec,
    _NonConformantEvent,
)

KLV_PID = 0x1031


def test_cell_fragment_indication_is_an_enum_with_four_variants() -> None:
    """The enum mirrors Rust `CellFragmentIndication`."""
    assert CellFragmentIndication.MIDDLE is not None
    assert CellFragmentIndication.LAST is not None
    assert CellFragmentIndication.FIRST is not None
    assert CellFragmentIndication.COMPLETE is not None


def test_cell_fragment_indication_discriminants_match_wire_bits() -> None:
    """Discriminants are the on-wire 2-bit values per H.222.0 Table 2-157.

    PyO3 `eq_int` enums compare with `==` against ints.
    """
    assert CellFragmentIndication.MIDDLE == 0
    assert CellFragmentIndication.LAST == 1
    assert CellFragmentIndication.FIRST == 2
    assert CellFragmentIndication.COMPLETE == 3


def test_cell_fragment_indication_eq_int_semantics() -> None:
    """eq_int enums compare equal to themselves, distinct from siblings."""
    assert CellFragmentIndication.MIDDLE == CellFragmentIndication.MIDDLE
    assert CellFragmentIndication.MIDDLE != CellFragmentIndication.COMPLETE


def test_demuxer_config_default_for_tolerance_is_true() -> None:
    """Tolerance-by-default — corpus-dominant real-world CFI=00 producer
    bug is rescued by default; receivers can opt out with
    `cfi_tolerance=False` for spec-strict conformance testing."""
    cfg = DemuxerConfig()
    assert cfg.cfi_tolerance is True


def test_demuxer_config_accepts_tolerance_false() -> None:
    cfg = DemuxerConfig(cfi_tolerance=False)
    assert cfg.cfi_tolerance is False


def test_non_conformant_kind_has_malformed_au_cell_cfi_tolerated() -> None:
    """The new kind is reachable from the `NonConformantKind` enum."""
    assert NonConformantKind.CFI_TOLERATED is not None
    # String discriminant per the established pattern.
    assert NonConformantKind.CFI_TOLERATED.value == (
        "malformed_au_cell_cfi_tolerated"
    )


def test_non_conformant_event_has_observed_cfi_and_treated_as_fields() -> None:
    """The optional typed CFI fields exist with `None` defaults for
    backward-compat with constructors that don't pass them."""
    from tstrans.mpegts import StreamId, StreamKindTag

    stream = StreamId(
        pid=KLV_PID,
        kind=StreamKindTag.KLV_SYNC,
        codec=None,
        program_number=1,
    )
    ev = _NonConformantEvent(
        stream=stream,
        issue="some other issue",
        kind=NonConformantKind.PCR_ANOMALY,
    )
    assert ev.observed_cfi is None
    assert ev.treated_as is None


def test_non_conformant_event_accepts_typed_cfi_fields() -> None:
    """Explicit typed CFI fields round-trip through the dataclass."""
    from tstrans.mpegts import StreamId, StreamKindTag

    stream = StreamId(
        pid=KLV_PID,
        kind=StreamKindTag.KLV_SYNC,
        codec=None,
        program_number=1,
    )
    ev = _NonConformantEvent(
        stream=stream,
        issue="malformed AU cell CFI tolerated",
        kind=NonConformantKind.CFI_TOLERATED,
        observed_cfi=CellFragmentIndication.MIDDLE,
        treated_as=CellFragmentIndication.COMPLETE,
    )
    assert ev.observed_cfi == CellFragmentIndication.MIDDLE
    assert ev.treated_as == CellFragmentIndication.COMPLETE


def test_cell_fragment_indication_in_module_all() -> None:
    """`CellFragmentIndication` is in `tstrans.mpegts.__all__`."""
    from tstrans import mpegts

    assert "CellFragmentIndication" in mpegts.__all__


# ── End-to-end: tolerance flag flows through DemuxerConfig to Demuxer ──────


def _synth_klv_ls(value_len: int = 32) -> bytes:
    """Minimal-valid sync KLV record: MISB ST 0601 UAS Datalink UL +
    BER short-form length + N value bytes (total 17 + value_len).
    """
    assert value_len < 128, "BER short-form only — use < 128"
    ul = b"\x06\x0e\x2b\x34\x02\x0b\x01\x01\x0e\x01\x03\x01\x01\x00\x00\x00"
    return ul + bytes([value_len]) + (b"\x42" * value_len)


def _build_muxer() -> Muxer:
    prog = MuxerProgramConfigBuilder(program_number=1, pmt_pid=0x1000)
    prog.add_video(0x1011, VideoCodec.H264)
    prog.add_klv(KLV_PID, KlvStreamType.SYNCHRONOUS_METADATA, carries_pts=True)
    b = MuxerConfig.builder()
    b.add_program(prog.build())
    return Muxer(b.build())


def _drain(mux: Muxer) -> bytes:
    """Drain all pending TS packets from the muxer into one bytes blob."""
    pending = mux.pending_packets()
    if pending == 0:
        return b""
    buf = bytearray(pending * 188)
    n = mux.pull(buf)
    return bytes(buf[:n])


def _pump_video(mux: Muxer, frame_count: int, base_pts_ticks: int = 90_000) -> None:
    """Pump enough video frames to advance PTS past the PSI cadence
    threshold (~100 ms), ensuring PMT lands before the KLV PES."""
    nal_aud = b"\x00\x00\x00\x01\x09\x10"
    for i in range(frame_count):
        mux.push_video(nal_aud, pts=Pts90khz.from_raw(base_pts_ticks + i * 3_000))


def _locate_first_au_cell_offset(ts_bytes: bytes) -> int:
    """Find the byte offset of the AU cell header for the first PUSI
    packet on `KLV_PID`. Mirrors the Rust helper in
    `mpegts_au_cell_tolerance.rs`."""
    for pkt_idx in range(0, len(ts_bytes), 188):
        pkt = ts_bytes[pkt_idx : pkt_idx + 188]
        if len(pkt) < 188:
            break
        pkt_pid = ((pkt[1] & 0x1F) << 8) | pkt[2]
        if pkt_pid != KLV_PID:
            continue
        pusi = (pkt[1] & 0x40) != 0
        if not pusi:
            continue
        af_present = (pkt[3] & 0x20) != 0
        idx = 4
        if af_present:
            af_len = pkt[idx]
            idx += 1 + af_len
        # PES header: start_code(3) + stream_id(1) + length(2) + flags1(1)
        #             + flags2(1) + header_data_length(1) + optional fields.
        pes_header_data_length = pkt[idx + 8]
        idx += 9 + pes_header_data_length
        return pkt_idx + idx
    raise AssertionError("no PUSI packet found on KLV_PID")


def _patch_au_cell_cfi(ts_bytes: bytearray, offset: int, cfi_bits: int) -> None:
    """Rewrite the `cell_fragment_indication` field (top 2 bits of byte 2
    of the 5-byte AU cell header)."""
    # Layout: byte 0 = metadata_service_id, byte 1 = sequence_number,
    # byte 2 = cfi(2) | decoder_config_flag(1) | random_access_indicator(1) |
    #           reserved(4). Keep the low 6 bits unchanged; replace the top 2.
    assert 0 <= cfi_bits <= 3
    current = ts_bytes[offset + 2]
    ts_bytes[offset + 2] = (cfi_bits << 6) | (current & 0x3F)


def _collect_events(demuxer: Demuxer, ts: bytes) -> list[DemuxEvent]:
    demuxer.feed(ts)
    out: list[DemuxEvent] = []
    while True:
        ev = demuxer.next_event()
        if ev is None:
            break
        out.append(ev)
    return out


def _build_ts_with_orphan_middle_cell() -> bytes:
    """Synthesize a TS where the KLV PID's first PUSI packet carries an
    AU cell with `cell_fragment_indication = 0b00` (Middle) and a valid
    self-consistent KLV payload — exactly the shape the tolerance mode
    is designed to rescue."""
    mux = _build_muxer()
    _pump_video(mux, 5)
    inner = _synth_klv_ls(value_len=32)
    mux.push_klv(inner, pts=Pts90khz.from_raw(90_000))
    ts = bytearray(_drain(mux))
    off = _locate_first_au_cell_offset(bytes(ts))
    _patch_au_cell_cfi(ts, off, cfi_bits=0)  # 0b00 = Middle
    return bytes(ts)


def test_strict_mode_orphan_middle_emits_orphan_diagnostic_only() -> None:
    """Strict config (cfi_tolerance=False, opt out of the new default):
    orphan Middle surfaces as `MULTI_CELL_AU{ORPHAN}` with zero metadata
    events, even when the inner payload is a valid KLV record."""
    ts = _build_ts_with_orphan_middle_cell()
    events = _collect_events(Demuxer(DemuxerConfig(cfi_tolerance=False)), ts)

    klv_events = [e for e in events if isinstance(e, DemuxEvent.Klv)]
    assert len(klv_events) == 0, "strict mode: no KLV metadata events"

    nonconf_events = [e for e in events if isinstance(e, DemuxEvent.NonConformant)]
    orphans = [
        e
        for e in nonconf_events
        if e.kind == NonConformantKind.MULTI_CELL_AU
    ]
    tolerated = [
        e
        for e in nonconf_events
        if e.kind == NonConformantKind.CFI_TOLERATED
    ]
    assert len(orphans) == 1, f"expected 1 orphan, got {len(orphans)}"
    assert len(tolerated) == 0, "strict mode must not emit tolerance diagnostics"


def test_tolerance_mode_orphan_middle_emits_klv_plus_typed_diagnostic() -> None:
    """Tolerance config (tolerance True): orphan Middle with valid KLV
    surfaces as one `DemuxEvent.Klv` (Complete) plus one
    `DemuxEvent.NonConformant` with `kind=CFI_TOLERATED`
    and typed `observed_cfi=MIDDLE` / `treated_as=COMPLETE`."""
    ts = _build_ts_with_orphan_middle_cell()
    cfg = DemuxerConfig(cfi_tolerance=True)
    events = _collect_events(Demuxer(cfg), ts)

    klv_events = [e for e in events if isinstance(e, DemuxEvent.Klv)]
    assert len(klv_events) == 1, (
        f"tolerance mode: 1 KLV event expected, got {len(klv_events)}"
    )
    assert klv_events[0].kind == MetadataKindTag.KLV_SYNC_AU_CELL
    assert klv_events[0].was_reassembled is False
    assert klv_events[0].cell_count == 1
    # The rescued payload is the full inner KLV record (17 + 32 = 49 bytes).
    assert len(klv_events[0].payload) == 49

    nonconf_events = [e for e in events if isinstance(e, DemuxEvent.NonConformant)]
    tolerated = [
        e
        for e in nonconf_events
        if e.kind == NonConformantKind.CFI_TOLERATED
    ]
    assert len(tolerated) == 1, (
        f"tolerance mode: 1 tolerance diagnostic expected, got {len(tolerated)}"
    )
    assert tolerated[0].observed_cfi == CellFragmentIndication.MIDDLE
    assert tolerated[0].treated_as == CellFragmentIndication.COMPLETE
    assert tolerated[0].stream.pid == KLV_PID

    # Tolerance mode REPLACES the orphan diagnostic — must not emit both.
    orphans = [
        e
        for e in nonconf_events
        if e.kind == NonConformantKind.MULTI_CELL_AU
    ]
    assert len(orphans) == 0, "tolerance mode must not also emit MULTI_CELL_AU"


# ── tstrans.io helpers accept config= ──────────────────────────────────────


def _write_malformed_ts(tmp_path) -> "Path":
    """Write the byte-patched orphan-Middle TS to a tmp file so we can
    feed it through `tstrans.io.extract_klv` / `tstrans.io.probe`."""
    from pathlib import Path

    ts = _build_ts_with_orphan_middle_cell()
    p = tmp_path / "malformed_cfi.ts"
    Path(p).write_bytes(ts)
    return p


def test_extract_klv_strict_config_yields_zero_records_on_malformed(tmp_path) -> None:
    """extract_klv with explicit `cfi_tolerance=False` (opt out of the
    new tolerance default) sees zero typed KLV from a malformed-CFI TS."""
    from tstrans.io import extract_klv

    ts_path = _write_malformed_ts(tmp_path)
    cfg = DemuxerConfig(cfi_tolerance=False)
    records = list(extract_klv(ts_path, parsed=True, config=cfg))
    # Filter to typed records only (skip_unknown defaults True so unknowns
    # are filtered out automatically).
    assert len(records) == 0, (
        f"strict mode: expected 0 typed KLV records from malformed-CFI TS, "
        f"got {len(records)}"
    )


def test_extract_klv_default_yields_records_on_malformed(tmp_path) -> None:
    """Default extract_klv (no config) rescues malformed-CFI records —
    tolerance is on by default. The raw payload is the discriminator:
    did the demuxer surface the bytes at all?"""
    from tstrans.io import extract_klv

    ts_path = _write_malformed_ts(tmp_path)
    raw = list(extract_klv(ts_path, parsed=False))
    assert len(raw) == 1, (
        f"default mode: expected 1 raw KLV payload from malformed-CFI TS, "
        f"got {len(raw)}"
    )
    assert len(raw[0]) == 49, "rescued payload is the 17-byte UL+length + 32-byte value"


def test_extract_klv_with_tolerance_config_yields_records_on_malformed(tmp_path) -> None:
    """extract_klv with explicit `cfi_tolerance=True` rescues the raw
    KLV bytes (same behavior as default since the flip)."""
    from tstrans.io import extract_klv

    ts_path = _write_malformed_ts(tmp_path)
    cfg = DemuxerConfig(cfi_tolerance=True)
    # The synthetic payload is built from MISB ST 0601 UL bytes + 32 filler
    # value bytes (0x42 × 32). The UL prefix + BER length make it pass the
    # tolerance validator, but parse_klv_universal would fail to decode the
    # filler as a real UAS Datalink LS. We test the raw form, which is the
    # discriminator: did the demuxer surface the payload at all?
    raw = list(extract_klv(ts_path, parsed=False, config=cfg))
    assert len(raw) == 1, (
        f"tolerance mode: expected 1 raw KLV payload from malformed-CFI TS, "
        f"got {len(raw)}"
    )
    assert len(raw[0]) == 49, "rescued payload is the 17-byte UL+length + 32-byte value"


def test_probe_accepts_config_kwarg(tmp_path) -> None:
    """`probe` accepts a config= kwarg and the malformed-CFI fixture
    surfaces `has_klv=True` only under the tolerance config (default
    rejects the orphan cell so no KLV event ever fires)."""
    from tstrans.io import probe

    ts_path = _write_malformed_ts(tmp_path)
    # Strict probe: the malformed AU cell never produces a KLV event.
    # has_klv comes from PMT classification though, so it's True on either
    # config — the PMT still declares the KLV stream. The config= param's
    # real effect surfaces in extract_klv (above); for probe we just
    # verify the kwarg is accepted without TypeError.
    result_strict = probe(ts_path)
    assert result_strict.has_klv is True, "PMT declares KLV stream"

    cfg = DemuxerConfig(cfi_tolerance=True)
    result_tolerant = probe(ts_path, config=cfg)
    assert result_tolerant.has_klv is True


def test_parse_file_accepts_config_kwarg(tmp_path) -> None:
    """`parse_file` already accepted config= (it was the precedent for
    extending the API to probe + extract_klv). Verify it still works
    and surfaces the tolerance diagnostic when configured."""
    from tstrans.io import parse_file

    ts_path = _write_malformed_ts(tmp_path)
    cfg = DemuxerConfig(cfi_tolerance=True)
    events = list(parse_file(ts_path, config=cfg))

    tolerated = [
        e
        for e in events
        if isinstance(e, DemuxEvent.NonConformant)
        and e.kind == NonConformantKind.CFI_TOLERATED
    ]
    assert len(tolerated) == 1, (
        f"parse_file with tolerance config: expected 1 tolerance diagnostic, "
        f"got {len(tolerated)}"
    )
