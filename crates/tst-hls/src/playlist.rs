//! m3u8 playlist writer (EXT-X-VERSION 6; LIVE / EVENT / VOD modes).

use crate::config::HlsMode;
use crate::segmenter::Segmenter;

/// Render the current playlist as m3u8 text.
///
/// `is_final` is true when called from `finish()` — appends `#EXT-X-ENDLIST`
/// for Event/Vod modes.
pub(crate) fn render(segmenter: &Segmenter, is_final: bool) -> String {
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

    match segmenter.mode() {
        HlsMode::Event => out.push_str("#EXT-X-PLAYLIST-TYPE:EVENT\n"),
        HlsMode::Vod => out.push_str("#EXT-X-PLAYLIST-TYPE:VOD\n"),
        HlsMode::Live => {}
    }

    for seg in segmenter.visible_segments() {
        out.push_str(&format!("#EXTINF:{:.3},\n", seg.duration.as_secs_f64()));
        out.push_str(&format!("{}\n", seg.filename));
    }

    if is_final && !matches!(segmenter.mode(), HlsMode::Live) {
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
}
