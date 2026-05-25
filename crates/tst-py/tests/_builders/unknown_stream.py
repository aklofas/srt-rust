"""Synthetic MPEG-TS builder for an unrecognized stream_type (Audit-2 #1).

Constructs a minimal TS bitstream containing:
  - One PAT packet (PID 0x0000) advertising a single program whose PMT
    lives on PID 0x0100.
  - One PMT packet (PID 0x0100) declaring a single elementary stream on
    PID 0x0101 with the caller-supplied (unknown) stream_type byte.
  - Seven PES packets (PID 0x0101) carrying the raw payload.

The demuxer's sync-ingress state machine requires 5-of-7 aligned 0x47
bytes before it considers the stream locked (SYNC_REACQ_N=5, M=7 per
sync_ingress.rs). Supplying seven full 188-byte packets starting with
0x47 satisfies that requirement unconditionally.

No Muxer API is used — the raw bytes are composed here so that a new
public add_private_stream() method is not needed (scope-limiter from the
audit-2 task-1 plan).
"""
from __future__ import annotations


_TS_PACKET_SIZE = 188
_SYNC_BYTE = 0x47


def _ts_packet(pid: int, payload: bytes, pusi: bool = False, cc: int = 0) -> bytes:
    """Build one 188-byte TS packet.

    Args:
        pid:     13-bit PID (0..=0x1FFF).
        payload: up to 184 bytes of TS payload (zero-padded if shorter).
        pusi:    True to set the Payload Unit Start Indicator bit.
        cc:      4-bit continuity counter.
    """
    assert len(payload) <= 184, f"payload too long: {len(payload)}"
    buf = bytearray(_TS_PACKET_SIZE)
    buf[0] = _SYNC_BYTE
    # byte 1: TEI=0, PUSI, TP=0, PID[12:8]
    buf[1] = (0x40 if pusi else 0x00) | ((pid >> 8) & 0x1F)
    # byte 2: PID[7:0]
    buf[2] = pid & 0xFF
    # byte 3: scrambling=0, no AF, payload present, CC
    buf[3] = 0x10 | (cc & 0x0F)
    # bytes 4..187: payload (zero-padded)
    buf[4 : 4 + len(payload)] = payload
    return bytes(buf)


def _pat_packet() -> bytes:
    """PAT (PID 0x0000): program 1 → PMT PID 0x0100.

    Table structure per ISO/IEC 13818-1 §2.4.4.3:
      table_id=0x00, section_syntax_indicator=1, section_length computed,
      transport_stream_id=0x0001, version=0, current=1, section 0/0.
    """
    # Body: one NIT entry (prog 0 → 0x0010 optional, skipped) +
    #       one program entry: program_number=1, PMT_PID=0x0100.
    prog_entry = bytes([
        0x00, 0x01,        # program_number = 1
        0xE1, 0x00,        # reserved(3)=111b | PMT_PID[12:8]=0x01, PMT_PID[7:0]=0x00
    ])
    body = prog_entry
    section_length = 5 + len(body) + 4  # 5 = fixed header tail, 4 = CRC32
    header = bytes([
        0x00,              # table_id = 0x00 (PAT)
        0xB0 | ((section_length >> 8) & 0x0F),  # section_syntax_indicator=1, reserved, length hi
        section_length & 0xFF,                   # length lo
        0x00, 0x01,        # transport_stream_id = 1
        0xC1,              # reserved(2)=11b, version=0, current_next=1
        0x00,              # section_number = 0
        0x00,              # last_section_number = 0
    ])
    section_no_crc = header + body
    crc = _crc32_mpeg(section_no_crc)
    section = section_no_crc + crc.to_bytes(4, "big")
    # pointer_field = 0x00 (section starts immediately)
    payload = bytes([0x00]) + section
    return _ts_packet(pid=0x0000, payload=payload, pusi=True, cc=0)


def _pmt_packet(stream_type: int) -> bytes:
    """PMT (PID 0x0100): program 1, PCR PID 0x0101, one ES entry.

    ES entry: stream_type=<caller>, elementary_PID=0x0101, no descriptors.
    """
    es_entry = bytes([
        stream_type & 0xFF,   # stream_type byte
        0xE1, 0x01,           # reserved(3)=111b | elementary_PID[12:8]=0x01, PID[7:0]=0x01
        0xF0, 0x00,           # reserved(4)=1111b, ES_info_length=0
    ])
    # PMT section body (after the fixed 8-byte section header fields):
    #   PCR_PID(2) + program_info_length(2, =0) + ES entries
    pcr_and_info = bytes([
        0xE1, 0x01,   # reserved(3)=111b | PCR_PID[12:8]=0x01, PCR_PID[7:0]=0x01
        0xF0, 0x00,   # reserved(4)=1111b | program_info_length hi=0, lo=0
    ])
    body = pcr_and_info + es_entry
    section_length = 5 + len(body) + 4  # 5 = fixed header tail, 4 = CRC32
    header = bytes([
        0x02,              # table_id = 0x02 (PMT)
        0xB0 | ((section_length >> 8) & 0x0F),
        section_length & 0xFF,
        0x00, 0x01,        # program_number = 1
        0xC1,              # reserved(2)=11b, version=0, current_next=1
        0x00,              # section_number = 0
        0x00,              # last_section_number = 0
    ])
    section_no_crc = header + body
    crc = _crc32_mpeg(section_no_crc)
    section = section_no_crc + crc.to_bytes(4, "big")
    payload = bytes([0x00]) + section  # pointer_field = 0
    return _ts_packet(pid=0x0100, payload=payload, pusi=True, cc=0)


def _pes_packets(payload: bytes) -> list[bytes]:
    """PES on PID 0x0101 carrying `payload`.

    Builds a PES header (stream_id=0xBD private_stream_1, PTS=0) and
    then fills as many 188-byte TS packets as needed.  Returns at least
    7 packets (repeating stuffing packets if necessary) so the demuxer
    can complete sync acquisition.

    The PES header structure per ISO/IEC 13818-1 §2.4.3.7:
      start_code_prefix(3) + stream_id(1) + PES_packet_length(2) +
      flags(2) + header_data_length(1) + PTS(5)
    """
    # PTS = 0, encoded as 5 bytes
    pts_bits = 0
    pts_b = bytes([
        0x21 | (((pts_bits >> 30) & 0x07) << 1),          # marker '0010', PTS[32:30], marker
        (pts_bits >> 22) & 0xFF,                            # PTS[29:22]
        0x01 | (((pts_bits >> 15) & 0x7F) << 1),           # PTS[21:15], marker
        (pts_bits >> 7) & 0xFF,                             # PTS[14:7]
        0x01 | ((pts_bits & 0x7F) << 1),                   # PTS[6:0], marker
    ])
    pes_header_after_startcode = bytes([
        0xBD,          # stream_id = private_stream_1
    ])
    pes_optional_header = bytes([
        0x80,          # marker(2)=10b, no flags
        0x80,          # PTS_DTS_flags=10b (PTS only), other flags=0
        0x05,          # PES_header_data_length = 5
    ]) + pts_b
    pes_payload = pes_optional_header + payload
    pes_packet_length = len(pes_payload)
    pes_start = (
        bytes([0x00, 0x00, 0x01])  # start_code_prefix
        + pes_header_after_startcode
        + pes_packet_length.to_bytes(2, "big")
        + pes_payload
    )

    # Split into TS packets (each carries up to 184 bytes of TS payload)
    ts_packets = []
    remaining = pes_start
    pusi = True
    cc = 0
    while remaining:
        chunk = remaining[:184]
        remaining = remaining[184:]
        ts_packets.append(_ts_packet(pid=0x0101, payload=chunk, pusi=pusi, cc=cc))
        pusi = False
        cc = (cc + 1) & 0x0F

    # Pad to at least 7 packets with null-like stuffing packets on PID 0x1FFF
    # so the sync state machine sees ≥ 7 aligned 0x47 bytes.
    while len(ts_packets) < 7:
        ts_packets.append(_ts_packet(pid=0x1FFF, payload=b"", cc=0))

    return ts_packets


def _crc32_mpeg(data: bytes) -> int:
    """CRC-32 with the MPEG-2 polynomial (same as ITU-T CRC-32/MPEG-2)."""
    crc = 0xFFFF_FFFF
    for byte in data:
        crc ^= byte << 24
        for _ in range(8):
            if crc & 0x8000_0000:
                crc = (crc << 1) ^ 0x0000_0004_C11D_B7
            else:
                crc <<= 1
            crc &= 0xFFFF_FFFF
    return crc


def build_unknown_stream_ts(*, stream_type: int, payload: bytes) -> bytes:
    """Build a complete TS bitstream with one PES carrying an unknown stream_type.

    Args:
        stream_type: PMT stream_type byte (e.g. 0x7F — user private).
                     The demuxer must not classify this as Video/Audio/
                     Subtitle/KLV; it should surface as UnknownSample.
        payload:     Raw PES payload bytes to carry (any length ≤ 65535
                     after PES header overhead).

    Returns:
        bytes: a valid multi-packet MPEG-TS bitstream, always a multiple
               of 188 bytes, containing ≥ 7 aligned 0x47 sync bytes.
    """
    packets: list[bytes] = []
    packets.append(_pat_packet())
    packets.append(_pmt_packet(stream_type))
    packets.extend(_pes_packets(payload))
    return b"".join(packets)
