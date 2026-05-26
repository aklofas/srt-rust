//! MP2T media selection for `RtspClient::setup_mp2t_auto` (lands Wave B).

use crate::error::RtspError;
use crate::sdp::{Sdp, SdpMedia};

/// Find the unique `m=` line whose `payload_types` contains `33` (MP2T,
/// per RFC 3551 §6).
///
/// # Errors
///
/// - [`RtspError::NoMp2tMedia`] if no m-line has PT=33.
/// - [`RtspError::MultipleMp2tMedia`] if more than one m-line has PT=33.
pub fn pick_mp2t(sdp: &Sdp) -> Result<&SdpMedia, RtspError> {
    let mut found: Option<&SdpMedia> = None;
    let mut count = 0;
    for m in &sdp.media {
        if m.payload_types.contains(&33) {
            count += 1;
            if count == 1 {
                found = Some(m);
            }
        }
    }
    match count {
        0 => Err(RtspError::NoMp2tMedia),
        1 => Ok(found.unwrap()),
        _ => Err(RtspError::MultipleMp2tMedia { count }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_media(media: &str, pts: &[u8]) -> SdpMedia {
        SdpMedia {
            media: media.to_string(),
            port: 0,
            protocol: "RTP/AVP".to_string(),
            payload_types: pts.to_vec(),
            connection: None,
            attributes: Vec::new(),
            control: None,
        }
    }

    #[test]
    fn picks_unique_mp2t() {
        let sdp = Sdp {
            media: vec![
                make_media("video", &[96]),
                make_media("application", &[33]),
                make_media("audio", &[97]),
            ],
            session_connection: None,
            session_name: String::new(),
        };
        let m = pick_mp2t(&sdp).unwrap();
        assert_eq!(m.media, "application");
    }

    #[test]
    fn rejects_no_mp2t() {
        let sdp = Sdp {
            media: vec![make_media("video", &[96])],
            session_connection: None,
            session_name: String::new(),
        };
        assert!(matches!(
            pick_mp2t(&sdp).unwrap_err(),
            RtspError::NoMp2tMedia
        ));
    }

    #[test]
    fn rejects_multiple_mp2t() {
        let sdp = Sdp {
            media: vec![
                make_media("application", &[33]),
                make_media("application", &[33]),
            ],
            session_connection: None,
            session_name: String::new(),
        };
        let e = pick_mp2t(&sdp).unwrap_err();
        assert!(matches!(e, RtspError::MultipleMp2tMedia { count: 2 }));
    }
}
