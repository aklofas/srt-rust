//! m3u8 playlist writer (EXT-X-VERSION 6; LIVE / EVENT / VOD modes).

use crate::config::HlsMode;
use crate::segmenter::Segmenter;

/// One segment entry in a [`PlaylistModel`].
pub(crate) struct SegmentRef {
    pub duration: std::time::Duration,
    pub uri: String,
}

/// Structured representation of an m3u8 playlist.
///
/// Built from a [`Segmenter`] via [`PlaylistModel::from_segmenter`] and
/// serialised to m3u8 text via [`PlaylistModel::render`].  This indirection
/// is the "LL-ready bone": EXT-X-PART / SERVER-CONTROL tags can be added to
/// the model and rendered in one place without touching any call sites.
pub(crate) struct PlaylistModel {
    pub version: u8,
    pub target_duration_secs: u64,
    pub media_sequence: u64,
    /// `EXT-X-PLAYLIST-TYPE` tag value.  `None` for Live (tag omitted).
    pub playlist_type: Option<HlsMode>,
    pub segments: Vec<SegmentRef>,
    /// Whether to append `#EXT-X-ENDLIST`.
    pub end_list: bool,
}

impl PlaylistModel {
    /// Lift all playlist-relevant state out of `segmenter`.
    pub(crate) fn from_segmenter(segmenter: &Segmenter, is_final: bool) -> Self {
        let mode = segmenter.mode();
        let playlist_type = match mode {
            HlsMode::Live => None,
            other => Some(other),
        };
        let segments = segmenter
            .visible_segments()
            .into_iter()
            .map(|s| SegmentRef {
                duration: s.duration,
                uri: s.filename,
            })
            .collect();
        PlaylistModel {
            version: 6,
            target_duration_secs: segmenter.target_duration_secs(),
            media_sequence: segmenter.media_sequence(),
            playlist_type,
            segments,
            end_list: is_final && !matches!(mode, HlsMode::Live),
        }
    }

    /// Serialise the model to m3u8 text, byte-for-byte identical to the
    /// previous free function.
    pub(crate) fn render(&self) -> String {
        let mut out = String::with_capacity(512);

        out.push_str("#EXTM3U\n");
        out.push_str(&format!("#EXT-X-VERSION:{}\n", self.version));
        out.push_str(&format!(
            "#EXT-X-TARGETDURATION:{}\n",
            self.target_duration_secs
        ));
        out.push_str(&format!("#EXT-X-MEDIA-SEQUENCE:{}\n", self.media_sequence));

        match self.playlist_type {
            Some(HlsMode::Event) => out.push_str("#EXT-X-PLAYLIST-TYPE:EVENT\n"),
            Some(HlsMode::Vod) => out.push_str("#EXT-X-PLAYLIST-TYPE:VOD\n"),
            _ => {}
        }

        for seg in &self.segments {
            out.push_str(&format!("#EXTINF:{:.3},\n", seg.duration.as_secs_f64()));
            out.push_str(&format!("{}\n", seg.uri));
        }

        if self.end_list {
            out.push_str("#EXT-X-ENDLIST\n");
        }

        out
    }
}

/// Render the current playlist as m3u8 text.
///
/// `is_final` is true when called from `finish()` — appends `#EXT-X-ENDLIST`
/// for Event/Vod modes.
pub(crate) fn render(segmenter: &Segmenter, is_final: bool) -> String {
    PlaylistModel::from_segmenter(segmenter, is_final).render()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::HlsConfig;

    fn fresh_segmenter(mode: HlsMode) -> Segmenter {
        let dir = std::env::temp_dir().join(format!(
            "hls-pl-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = HlsConfig {
            output_dir: dir,
            mode,
            playlist_window: 6,
            ..HlsConfig::default()
        };
        Segmenter::new(cfg).unwrap()
    }

    #[test]
    fn empty_playlist_is_well_formed() {
        let s = fresh_segmenter(HlsMode::Live);
        let pl = render(&s, false);
        assert!(pl.starts_with("#EXTM3U\n"));
        assert!(pl.contains("#EXT-X-VERSION:6"));
        assert!(pl.contains("#EXT-X-TARGETDURATION:"));
        assert!(pl.contains("#EXT-X-MEDIA-SEQUENCE:0"));
        assert!(!pl.contains("#EXT-X-ENDLIST"));
    }

    #[test]
    fn event_playlist_with_segments_includes_extinf() {
        let mut s = fresh_segmenter(HlsMode::Event);
        s.push_ts(&[0x47u8; 188]).unwrap();
        s.cut().unwrap();
        s.push_ts(&[0x47u8; 188]).unwrap();
        s.cut().unwrap();
        let pl = render(&s, false);
        assert!(pl.contains("#EXT-X-PLAYLIST-TYPE:EVENT"));
        assert!(pl.contains("segment_00000.ts"));
        assert!(pl.contains("segment_00001.ts"));
        assert_eq!(pl.matches("#EXTINF:").count(), 2);
    }

    #[test]
    fn endlist_only_on_final_and_non_live() {
        let mut s = fresh_segmenter(HlsMode::Vod);
        s.push_ts(&[0x47u8; 188]).unwrap();
        s.cut().unwrap();
        let pl_mid = render(&s, false);
        assert!(!pl_mid.contains("#EXT-X-ENDLIST"));
        let pl_end = render(&s, true);
        assert!(pl_end.contains("#EXT-X-ENDLIST"));

        let s_live = fresh_segmenter(HlsMode::Live);
        let pl_live = render(&s_live, true);
        assert!(!pl_live.contains("#EXT-X-ENDLIST"));
    }

    /// A segmenter holding two real, closed segments with deterministic
    /// media-PTS durations (2.000 s and 2.500 s). `push_ts` opens each
    /// segment before `cut_with_duration` closes it — cutting a segmenter
    /// with no open segment is a documented no-op, so the segments must be
    /// opened first or the fixture produces an empty history.
    fn segmenter_with_two_segments(mode: HlsMode) -> Segmenter {
        let dir = std::env::temp_dir().join(format!(
            "hls-pl-golden-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = HlsConfig {
            output_dir: dir,
            mode,
            segment_duration: std::time::Duration::from_secs(4),
            playlist_window: 6,
            ..HlsConfig::default()
        };
        let mut s = Segmenter::new(cfg).unwrap();
        s.push_ts(&[0u8; 188]).unwrap();
        s.cut_with_duration(Some(std::time::Duration::from_millis(2000)))
            .unwrap();
        s.push_ts(&[0u8; 188]).unwrap();
        s.cut_with_duration(Some(std::time::Duration::from_millis(2500)))
            .unwrap();
        s
    }

    // Golden byte-level tests: pin the exact m3u8 the renderer produces so any
    // drift in `render()`/`PlaylistModel` is caught. (The old differential test
    // compared `render()` against the identical `PlaylistModel::from_segmenter
    // (..).render()` expression — `render()` is literally that call — so it was
    // tautological and could never fail; its fixture also cut a fresh segmenter
    // with no open segment, producing zero segments.)

    #[test]
    fn live_playlist_renders_expected_bytes() {
        let s = segmenter_with_two_segments(HlsMode::Live);
        // Live: no PLAYLIST-TYPE tag, and no ENDLIST even when is_final.
        let expected = "#EXTM3U\n\
             #EXT-X-VERSION:6\n\
             #EXT-X-TARGETDURATION:4\n\
             #EXT-X-MEDIA-SEQUENCE:0\n\
             #EXTINF:2.000,\n\
             segment_00000.ts\n\
             #EXTINF:2.500,\n\
             segment_00001.ts\n";
        assert_eq!(render(&s, false), expected);
        assert_eq!(render(&s, true), expected, "Live ignores is_final");
    }

    #[test]
    fn vod_playlist_renders_type_and_conditional_endlist() {
        let s = segmenter_with_two_segments(HlsMode::Vod);
        let body = "#EXTM3U\n\
             #EXT-X-VERSION:6\n\
             #EXT-X-TARGETDURATION:4\n\
             #EXT-X-MEDIA-SEQUENCE:0\n\
             #EXT-X-PLAYLIST-TYPE:VOD\n\
             #EXTINF:2.000,\n\
             segment_00000.ts\n\
             #EXTINF:2.500,\n\
             segment_00001.ts\n";
        assert_eq!(render(&s, false), body, "no ENDLIST before finish");
        assert_eq!(
            render(&s, true),
            format!("{body}#EXT-X-ENDLIST\n"),
            "ENDLIST appended on finish"
        );
    }
}
