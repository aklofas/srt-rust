# Synthetic audio fixtures

Synthetic MPEG-TS captures with audio + video, used by integration
tests for `mpegts::mux` and `mpegts::demux` audio carriage.

Each fixture is ~3 seconds of:
- **Audio:** 440 Hz mono sine wave at 44.1 kHz (lavfi `sine`).
- **Video:** 320×240 H.264 baseline test pattern at 25 fps (lavfi
  `testsrc`).

The audio waveform choice is deliberate — sine is deterministic
(byte-identical regen across machines), codec-stable (tone survives
encoding and decoding without becoming noise), and easy to detect
corruption against. Static or silence in a decoded fixture means
something's broken.

## Conformant fixtures

| File | PMT `stream_type` | Codec | Notes |
|---|---|---|---|
| `mp2.ts` | `0x03` | MPEG-1 Layer II | ffmpeg `-c:a mp2` |
| `aac-adts.ts` | `0x0F` | AAC ADTS | ffmpeg `-c:a aac` (default for mpegts muxer) |
| `aac-latm.ts` | `0x11` | AAC LATM | ffmpeg `-c:a aac -mpegts_flags +latm` |
| `ac3.ts` | `0x81` | AC-3 | ffmpeg `-c:a ac3` (ATSC mode + registration descriptor) |

## Non-conformant fixtures (corpus quirks)

These exercise the `DemuxerConfig::treat_as` override path. They
mimic patterns observed in the local real-world corpus.

| File | PMT `stream_type` | Real codec | Why non-conformant |
|---|---|---|---|
| `mp3-conformant.ts` | `0x03` | MP3 (Layer III) | ffmpeg's default — Layer III on MPEG-1 stream_type. Used as the rewrite source. |
| `mp3-on-0xF1.ts` | `0xF1` | MP3 | User-private stream_type. Shotover-ARS encoder pattern (14 such streams in the local corpus). |
| `mp3-on-0x03.ts` | `0x03` | MP3 | MP3 mislabeled as Layer II (corpus has 2 such streams). |

The rewrites use `tsp -P pmt --remove-pid + --add-pid` to swap the
PMT entry's stream_type byte without touching the bitstream.

## Regeneration

```sh
./regen.sh
```

Requires ffmpeg 6.x and tsduck 3.x installed locally. Not run by CI;
fixtures are committed bytes.

## Verification

`regen.sh` prints a one-line summary per file (size + ffprobe codec
name + codec_tag). Cross-check against the table above.

For deeper PMT inspection: `tstables FILE.ts --pid 4096`.
