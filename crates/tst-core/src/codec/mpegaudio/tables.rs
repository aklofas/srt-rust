//! MPEG audio bitrate and sample-rate lookup tables.

use super::model::{Layer, Version};

// Bitrate table per ISO 11172-3 §2.4.2.3 Table 8 + ISO 13818-3 Table 5.
// Indexed by [column][bitrate_index]. Column selection is by
// (version, layer); see `bitrate_column` below. Index 0 = free format
// (rejected); index 15 = forbidden.
//
// Columns:
//   0 = MPEG-1 Layer I
//   1 = MPEG-1 Layer II
//   2 = MPEG-1 Layer III
//   3 = MPEG-2/2.5 Layer I
//   4 = MPEG-2/2.5 Layer II/III (shared column per ISO 13818-3 Table 5)
pub(super) const BITRATE_TABLE: [[u32; 16]; 5] = [
    // index: 0   1   2   3   4    5    6    7    8    9    10   11   12   13   14   15
    [
        0, 32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416, 448, 0,
    ], // V1L1
    [
        0, 32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384, 0,
    ], // V1L2
    [
        0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 0,
    ], // V1L3
    [
        0, 32, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256, 0,
    ], // V2L1
    [
        0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160, 0,
    ], // V2L2/L3
];

pub(super) fn bitrate_column(version: Version, layer: Layer) -> usize {
    match (version, layer) {
        (Version::Mpeg1, Layer::I) => 0,
        (Version::Mpeg1, Layer::II) => 1,
        (Version::Mpeg1, Layer::III) => 2,
        (_, Layer::I) => 3,
        (_, _) => 4, // V2/V2.5 Layer II + Layer III share column 4
    }
}

// Sample rate table per ISO 11172-3 §2.4.2.3 Table 9 + ISO 13818-3 Table 6.
// Indexed by [version][sample_rate_index]. Index 3 = reserved.
pub(super) const SAMPLE_RATE_TABLE: [[u32; 4]; 3] = [
    [44100, 48000, 32000, 0], // MPEG-1
    [22050, 24000, 16000, 0], // MPEG-2
    [11025, 12000, 8000, 0],  // MPEG-2.5
];
