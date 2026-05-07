//! Nearest-PTS pairing state machine.
//!
//! Realtime mode: video events trigger immediate pairing against KLV
//! history. Buffered mode (Task 3) adds a bounded video buffer with
//! lookahead drain.

use super::types::{KlvSample, MatchMode, PairerOutput, VideoSample};
use std::collections::VecDeque;
use tst_core::mpegts::demux::{DemuxEvent, SamplePayload};

pub(super) struct NearestState {
    video_pid: u16,
    klv_pid: u16,
    tolerance_ticks: i64,
    max_klv_history: usize,
    mode: MatchMode,
    klv_history: VecDeque<KlvEntry>,
    #[allow(dead_code)] // populated in Task 3 (Buffered mode)
    video_buffer: VecDeque<VideoSample>,
}

struct KlvEntry {
    sample: KlvSample,
    used: bool,
}

impl NearestState {
    pub(super) fn new(
        video_pid: u16,
        klv_pid: u16,
        tolerance_ticks: i64,
        max_klv_history: usize,
        mode: MatchMode,
    ) -> Self {
        Self {
            video_pid,
            klv_pid,
            tolerance_ticks,
            max_klv_history,
            mode,
            klv_history: VecDeque::with_capacity(max_klv_history),
            video_buffer: VecDeque::new(),
        }
    }

    pub(super) fn feed(&mut self, event: DemuxEvent) -> Vec<PairerOutput> {
        // Dispatch on event shape + configured PIDs. Anything that
        // doesn't match the expected video/KLV shape on the configured
        // PIDs falls through to PassThrough — see types::PairerOutput
        // doc comment for the rationale (misconfiguration tolerance).
        match event {
            DemuxEvent::Sample {
                stream,
                pts,
                dts,
                payload: SamplePayload::Video { codec, payload },
            } if stream.pid == self.video_pid => {
                let v = VideoSample { stream, pts, dts, codec, payload };
                self.handle_video(v)
            }
            DemuxEvent::Metadata {
                stream,
                pts,
                kind,
                payload,
            } if stream.pid == self.klv_pid => {
                let k = KlvSample { stream, pts, kind, payload };
                self.handle_klv(k)
            }
            other => vec![PairerOutput::PassThrough(other)],
        }
    }

    pub(super) fn flush(&mut self) -> Vec<PairerOutput> {
        // Filled in Task 6.
        Vec::new()
    }

    fn handle_video(&mut self, v: VideoSample) -> Vec<PairerOutput> {
        match self.mode {
            MatchMode::Realtime => self.match_video_against_history(v),
            MatchMode::Buffered { .. } => {
                // Filled in Task 3.
                self.match_video_against_history(v)
            }
        }
    }

    fn handle_klv(&mut self, k: KlvSample) -> Vec<PairerOutput> {
        let mut out = Vec::new();
        self.klv_history
            .push_back(KlvEntry { sample: k, used: false });
        if self.klv_history.len() > self.max_klv_history {
            if let Some(evicted) = self.klv_history.pop_front() {
                if !evicted.used {
                    out.push(PairerOutput::UnpairedKlv(evicted.sample));
                }
            }
        }
        out
    }

    /// Linear-scan history for nearest-PTS KLV; emit Paired (and mark
    /// the entry used) if within tolerance, else UnpairedVideo. Tie-
    /// break on equidistant entries: prefer the newer (higher index in
    /// the FIFO), matching the spec's "most recently sent telemetry
    /// wins" rule.
    fn match_video_against_history(&mut self, v: VideoSample) -> Vec<PairerOutput> {
        let mut best: Option<(usize, i64)> = None;
        for (i, entry) in self.klv_history.iter().enumerate() {
            let dist = (entry.sample.pts - v.pts).abs();
            match best {
                None => best = Some((i, dist)),
                // Strict `<` so a later equidistant entry wins (newer
                // telemetry preferred). The loop walks oldest-to-newest,
                // so the last equidistant entry naturally wins via
                // `<=` — but using `<=` here makes the intent explicit
                // even if iteration order changes.
                Some((_, best_dist)) if dist <= best_dist => best = Some((i, dist)),
                _ => {}
            }
        }
        match best {
            Some((i, dist)) if dist <= self.tolerance_ticks => {
                let entry = &mut self.klv_history[i];
                entry.used = true;
                let klv = entry.sample.clone();
                vec![PairerOutput::Paired { video: v, klv }]
            }
            _ => vec![PairerOutput::UnpairedVideo(v)],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tst_core::mpegts::demux::{
        MetadataKind, StreamId, StreamKind, VideoCodec, VideoPayload,
    };

    const VIDEO_PID: u16 = 0x100;
    const KLV_PID: u16 = 0x102;

    fn video_event(pts: i64) -> DemuxEvent {
        DemuxEvent::Sample {
            stream: StreamId {
                pid: VIDEO_PID,
                kind: StreamKind::Video(VideoCodec::H264),
            },
            pts,
            dts: None,
            payload: SamplePayload::Video {
                codec: VideoCodec::H264,
                payload: VideoPayload::Nals(Vec::new()),
            },
        }
    }

    fn klv_event(pts: i64) -> DemuxEvent {
        DemuxEvent::Metadata {
            stream: StreamId {
                pid: KLV_PID,
                kind: StreamKind::KlvAsync,
            },
            pts,
            kind: MetadataKind::KlvAsync,
            payload: vec![0xAA, 0xBB],
        }
    }

    fn nearest_realtime() -> NearestState {
        NearestState::new(VIDEO_PID, KLV_PID, 100, 4, MatchMode::Realtime)
    }

    #[test]
    fn matching_pts_emits_paired() {
        let mut s = nearest_realtime();
        let _ = s.feed(klv_event(50));
        let out = s.feed(video_event(50));
        assert_eq!(out.len(), 1);
        assert!(matches!(&out[0], PairerOutput::Paired { .. }));
    }

    #[test]
    fn delta_at_tolerance_accepts() {
        let mut s = nearest_realtime();
        let _ = s.feed(klv_event(0));
        let out = s.feed(video_event(100)); // |delta| == tolerance
        assert!(matches!(&out[0], PairerOutput::Paired { .. }));
    }

    #[test]
    fn delta_one_past_tolerance_rejects() {
        let mut s = nearest_realtime();
        let _ = s.feed(klv_event(0));
        let out = s.feed(video_event(101)); // |delta| == tolerance + 1
        assert!(matches!(&out[0], PairerOutput::UnpairedVideo(_)));
    }

    #[test]
    fn fifo_eviction_emits_unpaired_klv_when_unused() {
        let mut s = nearest_realtime();
        let _ = s.feed(klv_event(0));
        let _ = s.feed(klv_event(1));
        let _ = s.feed(klv_event(2));
        let _ = s.feed(klv_event(3));
        // 5th KLV evicts the oldest (PTS=0) which was never used.
        let out = s.feed(klv_event(4));
        assert_eq!(out.len(), 1);
        match &out[0] {
            PairerOutput::UnpairedKlv(k) => assert_eq!(k.pts, 0),
            _ => panic!("expected UnpairedKlv, got {:?}", out[0]),
        }
    }

    #[test]
    fn fifo_eviction_silent_when_used() {
        let mut s = nearest_realtime();
        let _ = s.feed(klv_event(0));
        // Use the KLV via a video pair.
        let _ = s.feed(video_event(0));
        // Fill remaining 3 slots.
        let _ = s.feed(klv_event(1));
        let _ = s.feed(klv_event(2));
        let _ = s.feed(klv_event(3));
        // 5th KLV evicts the oldest (PTS=0) which IS used → silent.
        let out = s.feed(klv_event(4));
        assert!(out.is_empty(), "expected no eviction emission, got {:?}", out);
    }

    #[test]
    fn video_before_any_klv_emits_unpaired_video() {
        let mut s = nearest_realtime();
        let out = s.feed(video_event(50));
        assert_eq!(out.len(), 1);
        assert!(matches!(&out[0], PairerOutput::UnpairedVideo(_)));
    }

    #[test]
    fn equidistant_tiebreak_prefers_newer_klv() {
        let mut s = nearest_realtime();
        let _ = s.feed(klv_event(0));
        let _ = s.feed(klv_event(100));
        // Video at PTS=50: equidistant to both KLVs (delta=50). Newer
        // (PTS=100) wins per the spec rule.
        let out = s.feed(video_event(50));
        match &out[0] {
            PairerOutput::Paired { klv, .. } => assert_eq!(klv.pts, 100),
            _ => panic!("expected Paired with klv.pts=100"),
        }
    }
}
