//! m3u8 playlist writer (EXT-X-VERSION 6; LIVE / EVENT / VOD modes).

use crate::config::HlsMode;
use crate::segmenter::Segmenter;

/// Render the current playlist as m3u8 text.
///
/// `is_final` is true when called from `finish()` — appends `#EXT-X-ENDLIST`
/// for Event/Vod modes.
pub(crate) fn render(segmenter: &Segmenter, is_final: bool) -> String {
    let mode = segmenter.mode();
    let mut out = String::with_capacity(512);

    out.push_str("#EXTM3U\n");
    out.push_str("#EXT-X-VERSION:6\n");
    out.push_str(&format!(
        "#EXT-X-TARGETDURATION:{}\n",
        segmenter.target_duration_secs()
    ));
    out.push_str(&format!(
        "#EXT-X-MEDIA-SEQUENCE:{}\n",
        segmenter.media_sequence()
    ));

    match mode {
        HlsMode::Event => out.push_str("#EXT-X-PLAYLIST-TYPE:EVENT\n"),
        HlsMode::Vod => out.push_str("#EXT-X-PLAYLIST-TYPE:VOD\n"),
        HlsMode::Live => {}
    }

    for seg in segmenter.visible_segments() {
        out.push_str(&format!("#EXTINF:{:.3},\n", seg.duration.as_secs_f64()));
        out.push_str(&format!("{}\n", seg.filename));
    }

    if is_final && !matches!(mode, HlsMode::Live) {
        out.push_str("#EXT-X-ENDLIST\n");
    }

    out
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

    // Golden byte-level tests: pin the exact m3u8 `render()` produces so any
    // drift is caught. (The old differential test compared `render()` against
    // the identical `PlaylistModel::from_segmenter(..).render()` expression
    // from the pre-collapse two-type split — `render()` was literally that
    // call — so it was tautological and could never fail; its fixture also
    // cut a fresh segmenter with no open segment, producing zero segments.)

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
