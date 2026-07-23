//! ST 0806.4 Table 8-1 tag schema. Tags 11/12/13 (nested sets) are
//! dispatched explicitly in decode/encode; this table covers the scalar
//! items and drives once-only duplicate detection.

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum RvtEncoding {
    U8,                        // tags 5, 8, 14, 18
    U16,                       // tags 3, 4, 6
    U24,                       // tags 16, 17, 20, 21 (3-byte BE unsigned)
    U32,                       // tags 1 (CRC), 7, 9
    U64,                       // tag 2
    Iso7 { max_bytes: usize }, // tags 10 (127), 15 (3), 19 (3)
    Nested,                    // tags 11, 12, 13 — repeatable, dispatched by tag id
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct RvtTagSpec {
    pub(super) id: u8,
    pub(super) name: &'static str,
    pub(super) encoding: RvtEncoding,
}

pub(super) const RVT_TAGS: &[RvtTagSpec] = &[
    RvtTagSpec {
        id: 1,
        name: "CRC 32",
        encoding: RvtEncoding::U32,
    },
    RvtTagSpec {
        id: 2,
        name: "User Defined Time Stamp",
        encoding: RvtEncoding::U64,
    },
    RvtTagSpec {
        id: 3,
        name: "Platform True Airspeed",
        encoding: RvtEncoding::U16,
    },
    RvtTagSpec {
        id: 4,
        name: "Platform Indicated Airspeed",
        encoding: RvtEncoding::U16,
    },
    RvtTagSpec {
        id: 5,
        name: "Telemetry Accuracy Indicator",
        encoding: RvtEncoding::U8,
    },
    RvtTagSpec {
        id: 6,
        name: "Frag Circle Radius",
        encoding: RvtEncoding::U16,
    },
    RvtTagSpec {
        id: 7,
        name: "Frame Code",
        encoding: RvtEncoding::U32,
    },
    RvtTagSpec {
        id: 8,
        name: "UAS LS Version Number",
        encoding: RvtEncoding::U8,
    },
    RvtTagSpec {
        id: 9,
        name: "Video Data rate",
        encoding: RvtEncoding::U32,
    },
    RvtTagSpec {
        id: 10,
        name: "Digital Video File Format",
        encoding: RvtEncoding::Iso7 { max_bytes: 127 },
    },
    RvtTagSpec {
        id: 11,
        name: "User Defined LS",
        encoding: RvtEncoding::Nested,
    },
    RvtTagSpec {
        id: 12,
        name: "Point of Interest LS",
        encoding: RvtEncoding::Nested,
    },
    RvtTagSpec {
        id: 13,
        name: "Area of Interest LS",
        encoding: RvtEncoding::Nested,
    },
    RvtTagSpec {
        id: 14,
        name: "MGRS Zone",
        encoding: RvtEncoding::U8,
    },
    RvtTagSpec {
        id: 15,
        name: "MGRS Latitude Band and Grid Square",
        encoding: RvtEncoding::Iso7 { max_bytes: 3 },
    },
    RvtTagSpec {
        id: 16,
        name: "MGRS Easting",
        encoding: RvtEncoding::U24,
    },
    RvtTagSpec {
        id: 17,
        name: "MGRS Northing",
        encoding: RvtEncoding::U24,
    },
    RvtTagSpec {
        id: 18,
        name: "MGRS Zone (Frame Center)",
        encoding: RvtEncoding::U8,
    },
    RvtTagSpec {
        id: 19,
        name: "MGRS Latitude Band and Grid Square (Frame Center)",
        encoding: RvtEncoding::Iso7 { max_bytes: 3 },
    },
    RvtTagSpec {
        id: 20,
        name: "MGRS Easting (Frame Center)",
        encoding: RvtEncoding::U24,
    },
    RvtTagSpec {
        id: 21,
        name: "MGRS Northing (Frame Center)",
        encoding: RvtEncoding::U24,
    },
];

pub(super) fn lookup(id: u8) -> Option<&'static RvtTagSpec> {
    RVT_TAGS.iter().find(|t| t.id == id)
}
