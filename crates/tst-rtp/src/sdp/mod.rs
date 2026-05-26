//! Thin wrapper around `sdp-types` for our DESCRIBE response parsing.

use crate::error::RtspError;

pub mod pick;

/// A parsed SDP document.
#[derive(Debug, Clone)]
pub struct Sdp {
    pub media: Vec<SdpMedia>,
    /// Session-level `c=` connection address, if present.
    pub session_connection: Option<String>,
    /// Session name from `s=` line.
    pub session_name: String,
}

/// A single `m=` media line from SDP.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct SdpMedia {
    /// E.g., "video", "audio", "application".
    pub media: String,
    /// Port from the `m=` line.
    pub port: u16,
    /// Protocol — "RTP/AVP" for plain UDP, "RTP/AVP/TCP" if SDP advertises TCP-interleaved.
    pub protocol: String,
    /// Payload types from the `m=` line. For MP2T, this contains 33.
    pub payload_types: Vec<u8>,
    /// Media-level connection address (`c=` line under this m=), falls
    /// back to session-level `c=` if absent.
    pub connection: Option<String>,
    /// Per-media attributes (`a=` lines under this m=).
    pub attributes: Vec<(String, Option<String>)>,
    /// The `a=control:` URL for this media — used in SETUP request URI.
    pub control: Option<String>,
}

impl Sdp {
    /// Parse SDP bytes. Wraps `sdp-types` and surfaces any parse error
    /// as `RtspError::BadSdp`.
    pub fn parse(bytes: &[u8]) -> Result<Self, RtspError> {
        let parsed = sdp_types::Session::parse(bytes).map_err(|e| RtspError::BadSdp {
            detail: format!("{e:?}"),
        })?;
        let session_connection = parsed
            .connection
            .as_ref()
            .map(|c| c.connection_address.clone());
        let session_name = parsed.session_name.clone();
        let media: Vec<SdpMedia> = parsed
            .medias
            .iter()
            .map(|m| {
                let payload_types: Vec<u8> = m
                    .fmt
                    .split_whitespace()
                    .filter_map(|s| s.parse().ok())
                    .collect();
                let connection = m.connections.first().map(|c| c.connection_address.clone());
                let attributes: Vec<(String, Option<String>)> = m
                    .attributes
                    .iter()
                    .map(|a| (a.attribute.clone(), a.value.clone()))
                    .collect();
                let control = attributes
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("control"))
                    .and_then(|(_, v)| v.clone());
                SdpMedia {
                    media: m.media.clone(),
                    port: m.port,
                    protocol: m.proto.clone(),
                    payload_types,
                    connection,
                    attributes,
                    control,
                }
            })
            .collect();
        Ok(Sdp {
            media,
            session_connection,
            session_name,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_SDP_SINGLE_MP2T: &[u8] = b"\
v=0\r\n\
o=- 0 0 IN IP4 192.0.2.1\r\n\
s=GimbalCam RTSP\r\n\
t=0 0\r\n\
m=application 0 RTP/AVP 33\r\n\
c=IN IP4 192.0.2.1\r\n\
a=control:trackID=0\r\n";

    const SAMPLE_SDP_MULTI: &[u8] = b"\
v=0\r\n\
o=- 0 0 IN IP4 192.0.2.1\r\n\
s=Camera with audio+video\r\n\
t=0 0\r\n\
m=video 0 RTP/AVP 96\r\n\
a=rtpmap:96 H264/90000\r\n\
a=control:trackID=0\r\n\
m=audio 0 RTP/AVP 97\r\n\
a=rtpmap:97 MPEG4-GENERIC/48000/2\r\n\
a=control:trackID=1\r\n";

    #[test]
    fn parse_single_mp2t() {
        let sdp = Sdp::parse(SAMPLE_SDP_SINGLE_MP2T).unwrap();
        assert_eq!(sdp.media.len(), 1);
        assert_eq!(sdp.media[0].media, "application");
        assert_eq!(sdp.media[0].payload_types, vec![33]);
        assert_eq!(sdp.media[0].control.as_deref(), Some("trackID=0"));
    }

    #[test]
    fn parse_multi_no_mp2t() {
        let sdp = Sdp::parse(SAMPLE_SDP_MULTI).unwrap();
        assert_eq!(sdp.media.len(), 2);
        assert_eq!(sdp.media[0].payload_types, vec![96]);
        assert_eq!(sdp.media[1].payload_types, vec![97]);
    }

    #[test]
    fn parse_malformed() {
        let bad = b"v="; // no value, missing CRLF
        let e = Sdp::parse(bad).unwrap_err();
        assert!(matches!(e, RtspError::BadSdp { .. }));
    }
}
