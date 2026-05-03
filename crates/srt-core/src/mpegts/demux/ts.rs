//! TS-packet unpacker — parse a single 188-byte transport-stream packet.
//!
//! Inverse of `mpegts::mux::ts::TsPacketWriter`. ISO/IEC 13818-1 §2.4.3.2.

/// Parsed fields from one TS packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TsPacket<'a> {
    pub pid: u16,
    pub payload_unit_start: bool,
    pub continuity_counter: u8,
    pub has_adaptation_field: bool,
    /// 27 MHz PCR value if the adaptation field carried one.
    pub pcr_27mhz: Option<u64>,
    /// Adaptation field's `discontinuity_indicator` flag.
    pub discontinuity_indicator: bool,
    /// Slice into the input bytes pointing at the payload (post-adaptation,
    /// post-pointer-field if PSI). Empty if `has_payload=false`.
    pub payload: &'a [u8],
    pub has_payload: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TsParseError {
    NoSyncByte,
    Truncated,
    /// Adaptation field claimed length that doesn't fit in the packet.
    BadAdaptationLength,
}

pub fn parse_ts_packet(buf: &[u8]) -> Result<TsPacket<'_>, TsParseError> {
    if buf.len() != 188 {
        return Err(TsParseError::Truncated);
    }
    if buf[0] != 0x47 {
        return Err(TsParseError::NoSyncByte);
    }
    let payload_unit_start = (buf[1] & 0x40) != 0;
    let pid = u16::from_be_bytes([buf[1] & 0x1F, buf[2]]);
    let adaptation_control = (buf[3] >> 4) & 0x03;
    let has_adaptation_field = adaptation_control & 0b10 != 0;
    let has_payload = adaptation_control & 0b01 != 0;
    let continuity_counter = buf[3] & 0x0F;
    let mut payload_off = 4;
    let mut pcr_27mhz = None;
    let mut discontinuity_indicator = false;
    if has_adaptation_field {
        let af_len = buf[4] as usize;
        if 5 + af_len > 188 {
            return Err(TsParseError::BadAdaptationLength);
        }
        if af_len >= 1 {
            let flags = buf[5];
            discontinuity_indicator = (flags & 0x80) != 0;
            let pcr_flag = (flags & 0x10) != 0;
            if pcr_flag && af_len >= 7 {
                let b = &buf[6..12];
                let base = (((b[0] as u64) << 25)
                    | ((b[1] as u64) << 17)
                    | ((b[2] as u64) << 9)
                    | ((b[3] as u64) << 1)
                    | (((b[4] as u64) >> 7) & 0x01))
                    & ((1u64 << 33) - 1);
                let ext = (((b[4] as u64) & 0x01) << 8) | (b[5] as u64);
                pcr_27mhz = Some(base * 300 + ext);
            }
        }
        payload_off = 5 + af_len;
    }
    let payload = if has_payload && payload_off < 188 {
        &buf[payload_off..188]
    } else {
        &[]
    };
    Ok(TsPacket {
        pid,
        payload_unit_start,
        continuity_counter,
        has_adaptation_field,
        pcr_27mhz,
        discontinuity_indicator,
        payload,
        has_payload,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_simple_packet(pid: u16, pusi: bool, cc: u8) -> [u8; 188] {
        let mut buf = [0xFFu8; 188];
        buf[0] = 0x47;
        buf[1] = if pusi { 0x40 } else { 0x00 } | ((pid >> 8) as u8 & 0x1F);
        buf[2] = (pid & 0xFF) as u8;
        buf[3] = 0x10 | (cc & 0x0F); // adaptation_control=01 (payload only)
        buf
    }

    #[test]
    fn parses_simple_packet() {
        let buf = build_simple_packet(0x100, true, 0xA);
        let pkt = parse_ts_packet(&buf).unwrap();
        assert_eq!(pkt.pid, 0x100);
        assert!(pkt.payload_unit_start);
        assert_eq!(pkt.continuity_counter, 0xA);
        assert!(pkt.has_payload);
        assert_eq!(pkt.payload.len(), 184);
    }

    #[test]
    fn rejects_no_sync_byte() {
        let mut buf = build_simple_packet(0x100, false, 0);
        buf[0] = 0x00;
        assert!(matches!(
            parse_ts_packet(&buf),
            Err(TsParseError::NoSyncByte)
        ));
    }

    #[test]
    fn rejects_short_buffer() {
        let buf = [0u8; 100];
        assert!(matches!(
            parse_ts_packet(&buf),
            Err(TsParseError::Truncated)
        ));
    }
}
