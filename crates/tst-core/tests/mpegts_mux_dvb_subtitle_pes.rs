//! Verifies DVB-sub PES_data_field auto-wrap per ETSI EN 300 743 §6.2.

use tst_core::mpegts::mux::{
    Muxer, MuxerConfig, MuxerProgramConfigBuilder, SubtitleCodec, VideoCodec,
};

/// Drain the muxer once and reassemble the PES payload (post-PES-header)
/// from every TS packet on `pid`. Skips adaptation field and the PES header
/// (PUSI packets only) to leave the codec-level PES_data_field bytes.
fn reassemble_pes_payload(mux: &mut Muxer, pid: u16) -> Vec<u8> {
    let mut ts_buf = vec![0u8; 1316 * 4];
    let n = mux.pull(&mut ts_buf);
    let mut pes_payload = Vec::new();
    for pkt in ts_buf[..n].chunks_exact(188) {
        let p = ((pkt[1] as u16 & 0x1F) << 8) | pkt[2] as u16;
        if p != pid {
            continue;
        }
        let pusi = (pkt[1] & 0x40) != 0;
        let af_present = (pkt[3] & 0x20) != 0;
        let mut idx = 4usize;
        if af_present {
            let af_len = pkt[idx] as usize;
            idx += 1 + af_len;
        }
        if pusi && idx + 9 <= 188 {
            let pes_header_data_length = pkt[idx + 8] as usize;
            idx += 9 + pes_header_data_length;
        }
        if idx < 188 {
            pes_payload.extend_from_slice(&pkt[idx..188]);
        }
    }
    pes_payload
}

#[test]
fn dvb_sub_push_emits_data_identifier_stream_id_segments_marker() {
    let cfg = {
        let mut prog = MuxerProgramConfigBuilder::new(1, 0x100);
        prog.add_video(0x101, VideoCodec::H264);
        prog.add_subtitle(
            0x200,
            SubtitleCodec::DvbSubtitling {
                language: *b"eng",
                subtitling_type: 0x10,
                composition_page_id: 1,
                ancillary_page_id: 1,
            },
        );
        let mut b = MuxerConfig::builder();
        b.add_program(prog.build());
        b.build().unwrap()
    };
    let mut mux = Muxer::new(cfg).unwrap();

    // Synthetic page_composition_segment per EN 300 743 §7.2.2 Table 9:
    //   sync_byte=0x0F + segment_type=0x10 + page_id BE u16 + segment_length BE u16 + body
    // segment_length=2: page_time_out(1) + page_state-byte(1), zero regions.
    let segment: Vec<u8> = vec![0x0F, 0x10, 0x00, 0x01, 0x00, 0x02, 0x00, 0x10];
    let h = mux.subtitle_handles()[0];
    mux.push_subtitle_to(h, 90_000, &segment).unwrap();

    let pes_payload = reassemble_pes_payload(&mut mux, 0x200);

    // EN 300 743 §6.2 PES_data_field shape:
    //   data_identifier=0x20 + subtitle_stream_id=0x00 + segments + marker=0xFF.
    assert_eq!(pes_payload[0], 0x20, "data_identifier per EN 300 743 §6.2");
    assert_eq!(
        pes_payload[1], 0x00,
        "subtitle_stream_id per EN 300 743 §6.2"
    );
    assert_eq!(&pes_payload[2..2 + segment.len()], &segment[..]);
    assert_eq!(
        pes_payload[2 + segment.len()],
        0xFF,
        "end_of_PES_data_field_marker per EN 300 743 §6.2"
    );
    assert_eq!(
        pes_payload.len(),
        2 + segment.len() + 1,
        "envelope = data_id + stream_id + segments + marker"
    );
}

#[test]
fn dvb_sub_multi_segment_push_chains_segments_between_envelope() {
    // Two synthetic segments back-to-back. Library must concatenate them
    // verbatim between the data_id/stream_id prefix and the 0xFF marker —
    // not interpret the bytes (e.g. wouldn't insert a second 0x20 between).
    let cfg = {
        let mut prog = MuxerProgramConfigBuilder::new(1, 0x100);
        prog.add_video(0x101, VideoCodec::H264);
        prog.add_subtitle(
            0x200,
            SubtitleCodec::DvbSubtitling {
                language: *b"eng",
                subtitling_type: 0x10,
                composition_page_id: 1,
                ancillary_page_id: 1,
            },
        );
        let mut b = MuxerConfig::builder();
        b.add_program(prog.build());
        b.build().unwrap()
    };
    let mut mux = Muxer::new(cfg).unwrap();

    let mut payload = Vec::new();
    payload.extend_from_slice(&[0x0F, 0x10, 0x00, 0x01, 0x00, 0x02, 0x00, 0x10]); // page_comp
    payload.extend_from_slice(&[0x0F, 0x80, 0x00, 0x01, 0x00, 0x00]); // end_of_display_set, length=0

    let h = mux.subtitle_handles()[0];
    mux.push_subtitle_to(h, 90_000, &payload).unwrap();

    let pes_payload = reassemble_pes_payload(&mut mux, 0x200);
    assert_eq!(pes_payload[0], 0x20);
    assert_eq!(pes_payload[1], 0x00);
    assert_eq!(&pes_payload[2..2 + payload.len()], &payload[..]);
    assert_eq!(pes_payload[2 + payload.len()], 0xFF);
}
