//! Read an MPEG-TS file, demux it, and print every subtitle Sample
//! event encountered. Demonstrates the receiver-side classification
//! cascade and how to handle each subtitle codec.
//!
//! Usage:
//!     cargo run -p tst-examples --example demux_subtitle_file -- input.ts
//!
//! Pair with `mux_with_webvtt_subtitles.rs`:
//!     cargo run -p tst-examples --example mux_with_webvtt_subtitles -- /tmp/test.ts
//!     cargo run -p tst-examples --example demux_subtitle_file        -- /tmp/test.ts
//!
//! What you'll see: one line per subtitle Sample event, tagged with
//! the classified codec, PID, PTS, and byte count. WebVTT cues also
//! get a one-line UTF-8 preview (cues are guaranteed UTF-8 per Apple's
//! HLS WebVTT-in-TS draft); the other three codecs are bitstream-shaped
//! and would need a typed parser to render — those parsers are not
//! shipped today.

use std::env;
use std::fs::File;
use std::io::Read;

use tst_core::mpegts::demux::{DemuxEvent, Demuxer, SamplePayload, SubtitleCodec};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = env::args()
        .nth(1)
        .ok_or("usage: demux_subtitle_file <input.ts>")?;
    let mut bytes = Vec::new();
    File::open(&path)?.read_to_end(&mut bytes)?;

    // Default `Demuxer::new()` opts in to lenient mode. The
    // classification cascade routes `stream_type 0x06` PIDs by PMT
    // descriptor priority (in order):
    //   subtitling_descriptor → DvbSubtitling
    //   teletext_descriptor   → DvbTeletext
    //   registration "VTTC"   → WebVttInTs
    //   registration "GA94"   → Cea708Standalone
    //   registration "KLVA"   → KlvAsync
    //   metadata_descriptor   → KlvSync
    //   (none of the above)   → Unknown(0x06) (lenient) /
    //                           strict-rejected on `StrictMode::Sync`+
    //
    // For non-conformant streams (e.g. WebVTT-shaped bytes on a
    // PID with no VTTC descriptor — encoder bug), use
    // `Demuxer::with_config(DemuxerConfig { stream_kind_overrides:
    // <map>, .. })` to force a specific PID to a specific kind.
    let mut demux = Demuxer::new();

    // Single-shot feed of the whole file. The demuxer accepts
    // arbitrary byte slices and re-syncs on TS packet boundaries
    // internally — no alignment requirement on the caller. Lenient
    // mode never errors on PSI / PES non-conformance — those surface
    // as inline `NonConformant` events. It can still return
    // `Unrecoverable` (no 0x47 sync byte within the search window;
    // i.e. input isn't TS at all) or `MalformedPes`, both fatal for
    // an offline triage tool.
    demux
        .feed(&bytes)
        .expect("input could not be decoded as MPEG-TS");

    // `flush` is the canonical end-of-stream signal. Subtitle PES
    // typically declare an explicit `PES_packet_length`, so they're
    // committed eagerly and `flush` is mostly a no-op for them — but
    // for video PES with `PES_packet_length=0` (the common shape for
    // AUs >65535 bytes) the trailing AU sits in the reassembler
    // until you call `flush`. Always call it at end-of-input.
    demux.flush();

    let mut subtitle_count = 0;
    while let Some(event) = demux.next_event() {
        // Subtitle Sample events are the only ones we care about
        // here; every other event variant (`ProgramMap`, video /
        // audio / KLV `Sample`, `Metadata`, `Discontinuity`,
        // `NonConformant`) gets ignored by the destructuring `if
        // let`. A real triage tool would handle the others — see
        // `demux_to_events.rs` for the exhaustive shape.
        if let DemuxEvent::Sample {
            stream,
            pts,
            payload: SamplePayload::Subtitle { codec, payload },
            ..
        } = event
        {
            subtitle_count += 1;
            // Match on the demux-side `SubtitleCodec` enum (variants
            // are param-less here, in contrast to the mux-side enum
            // which carries language/page-id fields for DVB
            // codecs — descriptor params surface separately on
            // `StreamInfo::raw_descriptors` if needed).
            let kind = match codec {
                SubtitleCodec::DvbSubtitling => "DVB-Subtitling",
                SubtitleCodec::DvbTeletext => "DVB-Teletext",
                SubtitleCodec::Cea708Standalone => "CEA-708 standalone",
                SubtitleCodec::WebVttInTs => "WebVTT-in-TS",
            };
            println!(
                "[{kind}] PID 0x{:04X} PTS {} ({} bytes)",
                stream.pid,
                pts.as_ticks(),
                payload.len()
            );
            // For WebVTT, the payload is guaranteed UTF-8 (Apple's
            // spec is explicit on encoding) — print the first line as
            // a preview. Other codecs are bitstream-shaped and would
            // need a typed parser (DVB-sub segments, teletext data
            // units, CEA-708 cc_data_pkts) — those parsers are
            // deferred to a follow-on session.
            if matches!(codec, SubtitleCodec::WebVttInTs) {
                let preview = String::from_utf8_lossy(&payload);
                println!(
                    "  payload[0]: {}",
                    preview.lines().next().unwrap_or("(empty)")
                );
            }
        }
    }

    println!("total subtitle Sample events: {subtitle_count}");
    Ok(())
}
