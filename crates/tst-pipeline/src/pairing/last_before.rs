//! Sample-and-hold (last-before-PTS) pairing state machine.
//!
//! Holds a single `Option<KlvSlot>`. Each video event pairs with the
//! current slot if `slot.pts <= video.pts` (and within freshness, if
//! configured). Past-only by definition — a future KLV (PTS > video
//! PTS) cannot satisfy "before."

use super::types::{KlvSample, PairerOutput, VideoSample};
use tst_core::mpegts::demux::{DemuxEvent, SamplePayload};

pub(super) struct LastBeforeState {
    video_pid: u16,
    klv_pid: u16,
    freshness_ticks: Option<i64>,
    current: Option<KlvSlot>,
}

struct KlvSlot {
    sample: KlvSample,
    used: bool,
}

impl LastBeforeState {
    pub(super) fn new(video_pid: u16, klv_pid: u16, freshness_ticks: Option<i64>) -> Self {
        Self {
            video_pid,
            klv_pid,
            freshness_ticks,
            current: None,
        }
    }

    pub(super) fn feed(&mut self, event: DemuxEvent) -> Vec<PairerOutput> {
        match event {
            DemuxEvent::Sample {
                stream,
                pts,
                dts,
                payload: SamplePayload::Video { codec, payload },
            } if stream.pid == self.video_pid => {
                let v = VideoSample {
                    stream,
                    pts,
                    dts,
                    codec,
                    payload,
                };
                self.handle_video(v)
            }
            DemuxEvent::Metadata {
                stream,
                pts,
                kind,
                payload,
            } if stream.pid == self.klv_pid => {
                let k = KlvSample {
                    stream,
                    pts,
                    kind,
                    payload,
                };
                self.handle_klv(k)
            }
            other => vec![PairerOutput::PassThrough(other)],
        }
    }

    pub(super) fn flush(&mut self) -> Vec<PairerOutput> {
        match self.current.take() {
            Some(slot) if !slot.used => vec![PairerOutput::UnpairedKlv(slot.sample)],
            _ => Vec::new(),
        }
    }

    fn handle_video(&mut self, v: VideoSample) -> Vec<PairerOutput> {
        let pair_eligible = self
            .current
            .as_ref()
            .map(|s| {
                s.sample.pts <= v.pts
                    && match self.freshness_ticks {
                        // PIPE-16 cross-ref: `v.pts - s.sample.pts` is safe
                        // because the gate above proves `s.sample.pts <= v.pts`
                        // (non-negative diff). No saturation needed.
                        Some(n) => v.pts - s.sample.pts <= n,
                        None => true,
                    }
            })
            .unwrap_or(false);
        if pair_eligible {
            // Mark used, clone the sample for emission.
            let slot = self.current.as_mut().unwrap();
            slot.used = true;
            let klv = slot.sample.clone();
            vec![PairerOutput::Paired { video: v, klv }]
        } else {
            vec![PairerOutput::UnpairedVideo(v)]
        }
    }

    fn handle_klv(&mut self, k: KlvSample) -> Vec<PairerOutput> {
        let prev = self.current.replace(KlvSlot {
            sample: k,
            used: false,
        });
        match prev {
            Some(slot) if !slot.used => vec![PairerOutput::UnpairedKlv(slot.sample)],
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tst_core::mpegts::demux::{MetadataKind, StreamId, StreamKind, VideoCodec, VideoPayload};

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

    #[test]
    fn paired_when_klv_before_video_no_freshness() {
        let mut s = LastBeforeState::new(VIDEO_PID, KLV_PID, None);
        let _ = s.feed(klv_event(50));
        let out = s.feed(video_event(100));
        assert!(matches!(&out[0], PairerOutput::Paired { .. }));
    }

    #[test]
    fn unpaired_when_klv_after_video() {
        let mut s = LastBeforeState::new(VIDEO_PID, KLV_PID, None);
        let _ = s.feed(klv_event(150));
        let out = s.feed(video_event(100));
        assert!(matches!(&out[0], PairerOutput::UnpairedVideo(_)));
    }

    #[test]
    fn freshness_at_threshold_pairs() {
        let mut s = LastBeforeState::new(VIDEO_PID, KLV_PID, Some(50));
        let _ = s.feed(klv_event(50));
        // delta == freshness → pair.
        let out = s.feed(video_event(100));
        assert!(matches!(&out[0], PairerOutput::Paired { .. }));
    }

    #[test]
    fn freshness_past_threshold_unpairs() {
        let mut s = LastBeforeState::new(VIDEO_PID, KLV_PID, Some(50));
        let _ = s.feed(klv_event(50));
        // delta == freshness + 1 → unpaired.
        let out = s.feed(video_event(101));
        assert!(matches!(&out[0], PairerOutput::UnpairedVideo(_)));
    }

    #[test]
    fn displaced_unused_klv_emits_unpaired() {
        let mut s = LastBeforeState::new(VIDEO_PID, KLV_PID, None);
        let _ = s.feed(klv_event(0));
        // Second KLV displaces the first (which was unused).
        let out = s.feed(klv_event(50));
        assert_eq!(out.len(), 1);
        match &out[0] {
            PairerOutput::UnpairedKlv(k) => assert_eq!(k.pts, 0),
            _ => panic!("expected UnpairedKlv(0), got {:?}", out[0]),
        }
    }

    #[test]
    fn displaced_used_klv_silent() {
        let mut s = LastBeforeState::new(VIDEO_PID, KLV_PID, None);
        let _ = s.feed(klv_event(0));
        let _ = s.feed(video_event(10)); // marks slot used
        let out = s.feed(klv_event(50));
        assert!(
            out.is_empty(),
            "expected silent displacement, got {:?}",
            out
        );
    }

    #[test]
    fn sample_and_hold_reuses_klv_for_many_videos() {
        let mut s = LastBeforeState::new(VIDEO_PID, KLV_PID, None);
        let _ = s.feed(klv_event(0));
        let p1 = s.feed(video_event(10));
        let p2 = s.feed(video_event(20));
        assert!(matches!(&p1[0], PairerOutput::Paired { .. }));
        assert!(matches!(&p2[0], PairerOutput::Paired { .. }));
    }

    #[test]
    fn flush_emits_unused_current_klv() {
        let mut s = LastBeforeState::new(VIDEO_PID, KLV_PID, None);
        let _ = s.feed(klv_event(0));
        let out = s.flush();
        assert_eq!(out.len(), 1);
        assert!(matches!(&out[0], PairerOutput::UnpairedKlv(_)));
    }

    #[test]
    fn flush_silent_when_current_used() {
        let mut s = LastBeforeState::new(VIDEO_PID, KLV_PID, None);
        let _ = s.feed(klv_event(0));
        let _ = s.feed(video_event(10));
        let out = s.flush();
        assert!(out.is_empty());
    }

    #[test]
    fn flush_idempotent() {
        let mut s = LastBeforeState::new(VIDEO_PID, KLV_PID, None);
        let _ = s.feed(klv_event(0));
        let _ = s.flush();
        let out2 = s.flush();
        assert!(out2.is_empty());
    }
}
