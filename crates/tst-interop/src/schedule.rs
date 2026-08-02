//! Shared event schedule: the single place `gen.rs`, `send.rs`, and
//! `serve.rs` each build the PTS-ordered list of pushes for one
//! [`Profile`]'s synthetic traffic. Each consumes the same schedule
//! differently — `gen::run` writes it as fast as the muxer/file allow
//! (no sleeps), `send::send_over_transport` and `serve::run_hls`/
//! `run_rtsp` pace it to wall-clock time — but the shape of the
//! traffic itself (which events, at which PTS, in which order) is
//! defined in exactly one place.

use crate::profiles::Profile;

/// 90 kHz ticks per second — the MPEG-TS PTS clock (ITU-T H.222.0 V9
/// §2.4.3.6).
pub(crate) const PTS_HZ: u32 = 90_000;

/// One scheduled push: a frame/record to push at its paired PTS tick
/// (see [`build_schedule`]'s returned tuples). Audio (when a profile
/// carries it) is paced 1:1 with video, reusing the video frame index as
/// its own sequence number — real AAC framing runs at ~43 Hz for
/// 1024-sample frames at 44.1 kHz, but nothing downstream checks audio
/// cadence (only presence), so pairing with video keeps the schedule
/// simple.
pub(crate) enum Event {
    Video { frame_idx: u32 },
    Klv { seq: u32 },
    Audio { frame_idx: u32 },
}

/// Build the PTS-ordered event schedule for `seconds` of profile `p`'s
/// synthetic traffic. Returns `(start_pts_ticks, events)` — `start` is
/// needed alongside the schedule by wall-clock-paced callers to compute
/// each event's target offset from the run's own start.
///
/// PTS advances `90_000 / p.fps` ticks per video frame and `90_000 /
/// p.klv_hz` ticks per KLV record, both from `p.start_pts_ticks`. Every
/// event for the whole window is computed up front, then sorted into
/// ascending PTS order, so streams sharing a muxer/sender/publisher see
/// traffic in the same relative time order a live pipeline would
/// (rather than "all video, then all KLV").
///
/// `two-program` profiles are handled by the caller pushing the same
/// video AU / KLV record onto every configured program's handles at the
/// same PTS — this schedule itself is program-agnostic.
pub(crate) fn build_schedule(p: &Profile, seconds: f64) -> (i64, Vec<(i64, Event)>) {
    let video_step_ticks = (PTS_HZ / p.fps) as i64;
    let klv_step_ticks = (PTS_HZ / p.klv_hz) as i64;
    let video_count = (seconds * p.fps as f64).round() as u32;
    let klv_count = (seconds * p.klv_hz as f64).round() as u32;
    let start = p.start_pts_ticks as i64;

    let mut events: Vec<(i64, Event)> =
        Vec::with_capacity(video_count as usize * (1 + p.audio as usize) + klv_count as usize);
    for i in 0..video_count {
        events.push((
            start + i as i64 * video_step_ticks,
            Event::Video { frame_idx: i },
        ));
        if p.audio {
            events.push((
                start + i as i64 * video_step_ticks,
                Event::Audio { frame_idx: i },
            ));
        }
    }
    for i in 0..klv_count {
        events.push((start + i as i64 * klv_step_ticks, Event::Klv { seq: i }));
    }
    events.sort_by_key(|(pts_ticks, _)| *pts_ticks);
    (start, events)
}
