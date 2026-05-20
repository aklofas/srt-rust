//! Tests for the `codec::aac` public iterator surface (frame-level).

use crate::codec::CodecParseError;
use crate::codec::aac::{
    AacChannelLayout, AacProfile, AdtsFrame, AdtsFrameOwned, MpegVersion, frames,
};

/// Helper: build full ADTS frame (7-byte header + zero-fill body).
/// Defaults: MPEG-2 ID, no CRC, AAC-LC profile, num_blocks_wire=0 (1 block).
fn build_frame(sample_rate_index: u8, channel_config: u8, total_len: u32) -> Vec<u8> {
    let mut h = vec![0u8; 7];
    h[0] = 0xFF;
    h[1] = 0b1111_0000 | (1 << 3) | 1; // ID=MPEG-2, layer=0, no CRC
    h[2] = (1 << 6) | ((sample_rate_index & 0xF) << 2) | ((channel_config >> 2) & 1); // LC profile
    h[3] = ((channel_config & 0b11) << 6) | (((total_len >> 11) & 0b11) as u8);
    h[4] = ((total_len >> 3) & 0xFF) as u8;
    h[5] = (((total_len & 0b111) as u8) << 5) | 0b1_1111;
    h[6] = 0b11_1111 << 2;
    let pad = total_len as usize - 7;
    let mut out = h;
    out.extend(std::iter::repeat(0u8).take(pad));
    out
}

#[test]
fn frames_empty_yields_none() {
    assert!(frames(&[]).next().is_none());
}

#[test]
fn frames_two_back_to_back() {
    let mut buf = build_frame(4, 2, 200);
    buf.extend(build_frame(4, 2, 200));
    let mut it = frames(&buf);
    let f1 = it.next().unwrap().unwrap();
    assert_eq!(f1.frame_length_bytes, 200);
    assert_eq!(f1.bytes().len(), 200);
    assert_eq!(f1.raw_header.len(), 7);
    let f2 = it.next().unwrap().unwrap();
    assert_eq!(f2.frame_length_bytes, 200);
    assert!(it.next().is_none());
}

#[test]
fn frames_truncated_body_yields_truncated() {
    let mut buf = build_frame(4, 2, 200);
    buf.truncate(50); // header decodes but body too short
    let mut it = frames(&buf);
    match it.next() {
        Some(Err(CodecParseError::Truncated { .. })) => {}
        other => panic!("expected Err(Truncated), got {:?}", other),
    }
    assert!(it.next().is_none());
}

#[test]
fn frames_short_header_yields_truncated() {
    let mut it = frames(&[0xFF, 0xFF]);
    match it.next() {
        Some(Err(CodecParseError::Truncated { needed: 7, had: 2 })) => {}
        other => panic!("expected Truncated 7,2, got {:?}", other),
    }
    assert!(it.next().is_none());
}

#[test]
fn frames_bad_sync_yields_bad_sync_word() {
    let bad = [0xAB; 7];
    let mut it = frames(&bad);
    match it.next() {
        Some(Err(CodecParseError::BadSyncWord { .. })) => {}
        other => panic!("expected BadSyncWord, got {:?}", other),
    }
    assert!(it.next().is_none());
}

#[test]
fn adts_frame_owned_roundtrip() {
    let body = vec![0xAA, 0xBB, 0xCC];
    let raw_header = vec![0x01, 0x02];
    let borrowed = AdtsFrame {
        profile: AacProfile::Lc,
        sample_rate_hz: 44100,
        channel_configuration: 2,
        channel_layout: AacChannelLayout::Channels(2),
        frame_length_bytes: 3,
        samples_per_frame: 1024,
        num_raw_data_blocks: 1,
        has_crc: false,
        mpeg_version: MpegVersion::Mpeg4,
        raw_header: raw_header.clone(),
        body: &body,
    };
    let owned = borrowed.to_owned();
    let reborrowed = owned.as_ref();
    assert_eq!(borrowed, reborrowed);
    assert_eq!(owned.body, vec![0xAA, 0xBB, 0xCC]);
    // C7 — `.channels()` returns `Some(2)` for canonical layouts.
    assert_eq!(borrowed.channels(), Some(2));
    assert_eq!(owned.channels(), Some(2));
    // Verify AdtsFrameOwned is constructible (all fields present)
    let _ = AdtsFrameOwned {
        profile: AacProfile::Lc,
        sample_rate_hz: 44100,
        channel_configuration: 2,
        channel_layout: AacChannelLayout::Channels(2),
        frame_length_bytes: 3,
        samples_per_frame: 1024,
        num_raw_data_blocks: 1,
        has_crc: false,
        mpeg_version: MpegVersion::Mpeg4,
        raw_header: vec![],
        body: vec![],
    };
}

/// C7 — iterator continues past a frame with `channel_configuration == 0`
/// (PCE-defined). Previously `decode_channels(0)` returned `ReservedValue`
/// which terminated the iterator and dropped every subsequent frame.
#[test]
fn frames_iterator_continues_past_pce_defined_channel_configuration() {
    // Three back-to-back frames: first uses `channel_configuration == 0`
    // (PCE-defined), the second and third use canonical stereo.
    let mut buf = build_frame(4, 0, 200);
    buf.extend(build_frame(4, 2, 200));
    buf.extend(build_frame(4, 2, 200));
    let mut it = frames(&buf);

    let f1 = it.next().unwrap().unwrap();
    assert_eq!(f1.channel_configuration, 0);
    assert_eq!(f1.channel_layout, AacChannelLayout::PceDefined);
    assert_eq!(f1.channels(), None, "PceDefined -> None");

    let f2 = it.next().unwrap().unwrap();
    assert_eq!(f2.channel_layout, AacChannelLayout::Channels(2));
    assert_eq!(f2.channels(), Some(2));

    let f3 = it.next().unwrap().unwrap();
    assert_eq!(f3.channel_layout, AacChannelLayout::Channels(2));

    assert!(it.next().is_none());
}
