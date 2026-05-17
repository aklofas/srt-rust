//! TS-packet unpacker — parse a single 188-byte transport-stream packet.
//!
//! Inverse of `mpegts::mux::ts::TsPacketWriter`. ISO/IEC 13818-1 §2.4.3.2.

/// Parsed fields from one TS packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TsPacket<'a> {
    pub pid: u16,
    /// `transport_error_indicator` per ISO/IEC 13818-1 §2.4.3.2 (bit 0x80 of
    /// byte 1). Set when an upstream link-layer (e.g. ATSC FEC, satellite
    /// demod, CMTS) flagged the packet as known-corrupt. The demuxer drops
    /// these packets per ffmpeg `mpegts.c:3091-3097`.
    pub transport_error_indicator: bool,
    pub payload_unit_start: bool,
    pub continuity_counter: u8,
    pub has_adaptation_field: bool,
    /// 27 MHz PCR value if the adaptation field carried one.
    pub pcr_27mhz: Option<u64>,
    /// Adaptation field's `discontinuity_indicator` flag.
    pub discontinuity_indicator: bool,
    /// Adaptation field's `random_access_indicator` flag, per ISO/IEC
    /// 13818-1 §2.4.3.4 flags byte bit 6 (0x40). Set by encoders and
    /// muxers on TS packets that begin an access unit decodable without
    /// information from previous AUs (IDR, CRA, etc.). The signal is
    /// independent of NAL-level type and reflects the stream-level
    /// random-access contract. False when no adaptation field is present.
    pub random_access_indicator: bool,
    /// Slice into the input bytes pointing at the payload (post-adaptation,
    /// post-pointer-field if PSI). Empty if `has_payload=false`.
    pub payload: &'a [u8],
    pub has_payload: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
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
    let transport_error_indicator = (buf[1] & 0x80) != 0;
    let payload_unit_start = (buf[1] & 0x40) != 0;
    let pid = u16::from_be_bytes([buf[1] & 0x1F, buf[2]]);
    let adaptation_control = (buf[3] >> 4) & 0x03;
    let has_adaptation_field = adaptation_control & 0b10 != 0;
    let has_payload = adaptation_control & 0b01 != 0;
    let continuity_counter = buf[3] & 0x0F;
    let mut payload_off = 4;
    let mut pcr_27mhz = None;
    let mut discontinuity_indicator = false;
    let mut random_access_indicator = false;
    if has_adaptation_field {
        let af_len = buf[4] as usize;
        if 5 + af_len > 188 {
            return Err(TsParseError::BadAdaptationLength);
        }
        if af_len >= 1 {
            let flags = buf[5];
            discontinuity_indicator = (flags & 0x80) != 0;
            random_access_indicator = (flags & 0x40) != 0;
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
        transport_error_indicator,
        payload_unit_start,
        continuity_counter,
        has_adaptation_field,
        pcr_27mhz,
        discontinuity_indicator,
        random_access_indicator,
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
        assert!(!pkt.transport_error_indicator);
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

    #[test]
    fn adaptation_field_random_access_indicator_extracted() {
        // Build a TS packet with adaptation field flags byte 0x40 (RAI set).
        let mut buf = [0xFFu8; 188];
        buf[0] = 0x47;
        buf[1] = 0x40 | ((0x100u16 >> 8) as u8 & 0x1F); // pusi=1, pid=0x100
        buf[2] = 0x00;
        buf[3] = 0x30; // adaptation_control=11 (af + payload)
        buf[4] = 1; // adaptation_field_length=1 (flags byte only)
        buf[5] = 0x40; // flags: RAI=1
        let pkt = parse_ts_packet(&buf).unwrap();
        assert!(pkt.random_access_indicator);
        assert!(!pkt.discontinuity_indicator);
    }

    #[test]
    fn adaptation_field_random_access_indicator_clear_when_unset() {
        // Build a TS packet with adaptation field flags byte 0x00.
        let mut buf = [0xFFu8; 188];
        buf[0] = 0x47;
        buf[1] = 0x40;
        buf[2] = 0x00;
        buf[3] = 0x30;
        buf[4] = 1;
        buf[5] = 0x00; // flags: all clear
        let pkt = parse_ts_packet(&buf).unwrap();
        assert!(!pkt.random_access_indicator);
    }

    #[test]
    fn no_adaptation_field_means_rai_false() {
        // Default-built packet: adaptation_control=01, no af present.
        let buf = build_simple_packet(0x100, true, 0);
        let pkt = parse_ts_packet(&buf).unwrap();
        assert!(!pkt.random_access_indicator);
    }
}
