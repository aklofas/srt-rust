//! Shared per-stream stats shape used by `mpegts::mux::MuxerStats` and
//! `mpegts::demux::DemuxerStats`. Identity is the PID; kind/codec lives
//! in `stream_type`. Same shape across sender and receiver sites so the
//! `srt-c` ABI is one struct + one fixed-size array.

/// Per-stream counters. Used at every site that emits or receives TS
/// elementary streams. PID is identity; `stream_type` is the PMT byte
/// (or `0x00` for PSI PIDs); `label` is None unless a PMT user-label
/// descriptor or a hardcoded PSI label populates it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StreamStats {
    pub pid: u16,
    pub stream_type: u8,
    /// Program number from the PAT/PMT that owns this stream. `0` for PSI
    /// PIDs (PAT, PMT) and for streams that were created before a PMT arrived.
    pub program_number: u16,
    pub label: Option<String>,
    pub items: u64,
    pub bytes: u64,
    pub discontinuities: u64,
}

/// Mux-side codec label. Used to populate [`StreamStats::label`] for
/// subtitle PIDs at `Muxer::new` time. Static labels cover the four
/// supported subtitle codecs:
/// * `DvbSubtitling` → `"DVB-Subtitling"`
/// * `DvbTeletext`   → `"DVB-Teletext"`
/// * `Cea708Standalone` → `"CEA-708-Standalone"`
/// * `WebVttInTs`    → `"WebVTT-in-TS"`
pub fn subtitle_codec_label(codec: &crate::mpegts::mux::SubtitleCodec) -> &'static str {
    match codec {
        crate::mpegts::mux::SubtitleCodec::DvbSubtitling { .. } => "DVB-Subtitling",
        crate::mpegts::mux::SubtitleCodec::DvbTeletext { .. } => "DVB-Teletext",
        crate::mpegts::mux::SubtitleCodec::Cea708Standalone => "CEA-708-Standalone",
        crate::mpegts::mux::SubtitleCodec::WebVttInTs => "WebVTT-in-TS",
    }
}

/// Demux-side codec label. Same labels as [`subtitle_codec_label`] but
/// over the param-less demux-side enum `mpegts::demux::event::SubtitleCodec`.
pub fn demux_subtitle_codec_label(
    codec: crate::mpegts::demux::event::SubtitleCodec,
) -> &'static str {
    use crate::mpegts::demux::event::SubtitleCodec;
    match codec {
        SubtitleCodec::DvbSubtitling => "DVB-Subtitling",
        SubtitleCodec::DvbTeletext => "DVB-Teletext",
        SubtitleCodec::Cea708Standalone => "CEA-708-Standalone",
        SubtitleCodec::WebVttInTs => "WebVTT-in-TS",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_all_zero() {
        let s = StreamStats::default();
        assert_eq!(s.pid, 0);
        assert_eq!(s.stream_type, 0);
        assert_eq!(s.label, None);
        assert_eq!(s.items, 0);
        assert_eq!(s.bytes, 0);
        assert_eq!(s.discontinuities, 0);
    }

    #[test]
    fn equality_is_field_wise() {
        let a = StreamStats {
            pid: 0x100,
            stream_type: 0x1B,
            program_number: 1,
            label: Some("EO".into()),
            items: 5,
            bytes: 1024,
            discontinuities: 0,
        };
        let b = a.clone();
        assert_eq!(a, b);
        let mut c = a.clone();
        c.items = 6;
        assert_ne!(a, c);
    }
}
