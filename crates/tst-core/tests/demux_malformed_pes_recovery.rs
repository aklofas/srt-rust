use tst_core::mpegts::demux::NonConformantIssue;

#[test]
fn malformed_pes_variant_exists() {
    let issue = NonConformantIssue::MalformedPes {
        pid: 0x100,
        reason: "test",
    };
    let _ = issue;
}
