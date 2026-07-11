//! RFC 6184 §8.2.1 SDP `a=fmtp` / `a=rtpmap` parsing for H.264.
//!
//! [`parse_rtpmap_h264`] discovers the dynamic payload type assigned to H.264
//! by scanning `a=rtpmap` lines. [`H264FmtpParams::parse`] then extracts the
//! packetization mode, sprop-parameter-sets NALUs, and profile-level-id from
//! the matching `a=fmtp` line.

use base64::Engine as _;
use base64::engine::DecodePaddingMode;
use base64::engine::general_purpose::{GeneralPurpose, GeneralPurposeConfig};
use tst_core::codec::h264::{parse_pps, parse_sps};

use crate::sdp::SdpMedia;

/// Base64 engine for `sprop-parameter-sets`: standard alphabet,
/// tolerant of missing trailing padding. RFC 6184 §8.2.1 mandates
/// base64, and real-world cameras and RTSP servers commonly omit the
/// `=` padding; a skipped sprop entry silently costs the stream its
/// out-of-band SPS/PPS. Invalid characters still fail the decode.
const SPROP_BASE64: GeneralPurpose = GeneralPurpose::new(
    &base64::alphabet::STANDARD,
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

/// Parameters extracted from the `a=fmtp` line for an H.264 media section.
///
/// If the `a=fmtp` line is absent or malformed, all fields carry their RFC
/// 6184 §8.2.1 defaults: `packetization_mode = 0`, empty
/// `sprop_parameter_sets`, no `profile_level_id`.
///
/// Marked non-exhaustive: RFC 6184 §8.2.1 defines further optional
/// parameters (e.g. `max-fs`, `max-mbps`) this struct may grow to carry.
/// Construct via [`H264FmtpParams::parse`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct H264FmtpParams {
    /// Packetization mode (RFC 6184 §5); 0 = Single NAL unit, 1 = Non-interleaved.
    /// Defaults to 0 when the fmtp line is absent.
    pub packetization_mode: u8,
    /// Decoded sprop-parameter-sets NALUs (each element is one raw NALU including
    /// its header byte). Only entries with a valid NALU type (SPS=7 or PPS=8)
    /// and clear forbidden-zero bit are kept.
    pub sprop_parameter_sets: Vec<Vec<u8>>,
    /// Verbatim `profile-level-id` hex string from the fmtp line, if present.
    pub profile_level_id: Option<String>,
}

impl H264FmtpParams {
    /// Parse H.264 fmtp parameters from an [`SdpMedia`] for a given payload
    /// type `pt`.
    ///
    /// This is infallible: missing or malformed fmtp data produces defaults
    /// rather than errors. Individual malformed sprop entries are skipped with
    /// a [`tracing::warn!`] log.
    pub fn parse(media: &SdpMedia, pt: u8) -> Self {
        let mut out = Self {
            packetization_mode: 0,
            sprop_parameter_sets: Vec::new(),
            profile_level_id: None,
        };

        // Find the `a=fmtp` attribute whose leading token matches `pt`.
        let fmtp_value = media
            .attributes
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case("fmtp"))
            .find_map(|(_, v)| {
                let v = v.as_deref()?;
                // Leading token is the payload type number.
                let mut parts = v.splitn(2, ' ');
                let pt_tok = parts.next()?.trim();
                let rest = parts.next()?.trim();
                let parsed_pt: u8 = pt_tok.parse().ok()?;
                if parsed_pt == pt { Some(rest) } else { None }
            });

        let Some(params_str) = fmtp_value else {
            return out;
        };

        // Each semicolon-separated token is a key=value (or bare key).
        for token in params_str.split(';') {
            let token = token.trim();
            if token.is_empty() {
                continue;
            }
            // Split on the first '=' only.
            let (key, val) = if let Some(pos) = token.find('=') {
                (&token[..pos], token[pos + 1..].trim())
            } else {
                (token, "")
            };
            let key = key.trim();

            if key.eq_ignore_ascii_case("packetization-mode") {
                match val.parse::<u8>() {
                    Ok(m) if m <= 2 => out.packetization_mode = m,
                    Ok(m) => {
                        tracing::warn!(
                            value = m,
                            "H264 fmtp: packetization-mode out of range (RFC 6184 §8.2.1 allows 0–2), using default 0"
                        );
                    }
                    Err(_) => {
                        tracing::warn!(
                            value = val,
                            "H264 fmtp: invalid packetization-mode, using default 0"
                        );
                    }
                }
            } else if key.eq_ignore_ascii_case("sprop-parameter-sets") {
                for entry in val.split(',') {
                    let entry = entry.trim();
                    if entry.is_empty() {
                        continue;
                    }
                    // Decode base64 (standard alphabet; padding optional —
                    // see SPROP_BASE64) per RFC 6184 §8.2.1.
                    let nalu = match SPROP_BASE64.decode(entry) {
                        Ok(b) => b,
                        Err(_) => {
                            tracing::warn!(
                                entry,
                                "H264 fmtp: skipping sprop entry — base64 decode failed"
                            );
                            continue;
                        }
                    };
                    if nalu.is_empty() {
                        continue;
                    }
                    let byte0 = nalu[0];
                    let nal_type = byte0 & 0x1F;
                    // Forbidden-zero bit (F bit) must be clear; type must be
                    // SPS (7) or PPS (8) per RFC 6184 §8.2.1.
                    if byte0 & 0x80 != 0 || (nal_type != 7 && nal_type != 8) {
                        tracing::warn!(
                            byte0,
                            "H264 fmtp: skipping sprop entry — unexpected NALU type or F bit set"
                        );
                        continue;
                    }
                    // Best-effort structural sanity: parse the RBSP (strip the
                    // 1-byte header) to catch clearly corrupt data.  A parse
                    // failure is non-fatal: we warn and keep the entry because
                    // the depacketizer needs the raw bytes regardless of
                    // whether our SPS/PPS parser can decode them fully.
                    let rbsp = &nalu[1..];
                    if nal_type == 7 {
                        if let Err(e) = parse_sps(rbsp) {
                            tracing::warn!(
                                error = %e,
                                "H264 fmtp: SPS sanity parse failed (keeping entry)"
                            );
                        }
                    } else if let Err(e) = parse_pps(rbsp) {
                        tracing::warn!(
                            error = %e,
                            "H264 fmtp: PPS sanity parse failed (keeping entry)"
                        );
                    }
                    out.sprop_parameter_sets.push(nalu);
                }
            } else if key.eq_ignore_ascii_case("profile-level-id") {
                out.profile_level_id = Some(val.to_string());
            }
        }

        out
    }
}

/// Discover the dynamic RTP payload type assigned to H.264 in an SDP media
/// section by scanning `a=rtpmap` lines.
///
/// Returns the payload type of the **first** `a=rtpmap` line that:
/// - names `H264` (case-insensitive),
/// - specifies a clock rate of exactly `90000` (RFC 6184 §8.1 mandates
///   90 kHz; all downstream PTS arithmetic assumes this rate), **and**
/// - whose payload type is listed in `media.payload_types`.
///
/// Lines that name `H264` but carry a wrong or absent clock-rate token are
/// skipped with a [`tracing::warn!`] so that a non-conformant entry cannot
/// shadow a valid one that appears later in the attribute list. `None` is
/// returned only when no conformant H.264 rtpmap line exists.
pub fn parse_rtpmap_h264(media: &SdpMedia) -> Option<u8> {
    media
        .attributes
        .iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case("rtpmap"))
        .find_map(|(_, v)| {
            // Value format: "<pt> <encoding-name>/<clock-rate>[/<channels>]"
            let v = v.as_deref()?;
            let mut parts = v.splitn(2, ' ');
            let pt_tok = parts.next()?.trim();
            let enc_tok = parts.next()?.trim();
            let pt: u8 = pt_tok.parse().ok()?;

            // Split "<encoding-name>/<clock-rate>[/<channels>]".
            let mut enc_parts = enc_tok.splitn(3, '/');
            let enc_name = enc_parts.next()?;
            if !enc_name.eq_ignore_ascii_case("H264") {
                return None;
            }

            // Clock rate MUST be present and equal to 90000 (RFC 6184 §8.1).
            // A wrong or absent clock rate means the rtpmap is non-conformant;
            // skip it so it cannot shadow a valid later entry.
            match enc_parts.next() {
                Some("90000") => {}
                Some(rate) => {
                    tracing::warn!(
                        pt,
                        clock_rate = rate,
                        "H264 rtpmap: clock rate is not 90000 (RFC 6184 §8.1 mandates 90 kHz); \
                         skipping this entry"
                    );
                    return None;
                }
                None => {
                    tracing::warn!(
                        pt,
                        "H264 rtpmap: clock rate token absent (malformed rtpmap grammar); \
                         skipping this entry"
                    );
                    return None;
                }
            }

            // The payload type must be listed in the m= line.
            if media.payload_types.contains(&pt) {
                Some(pt)
            } else {
                None
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sdp::SdpMedia;

    fn media_with(attrs: &[(&str, &str)], pts: &[u8]) -> SdpMedia {
        SdpMedia {
            media: "video".into(),
            port: 0,
            protocol: "RTP/AVP".into(),
            payload_types: pts.to_vec(),
            connection: None,
            attributes: attrs
                .iter()
                .map(|(k, v)| (k.to_string(), Some(v.to_string())))
                .collect(),
            control: None,
        }
    }

    #[test]
    fn rtpmap_finds_dynamic_h264_pt() {
        let m = media_with(&[("rtpmap", "96 H264/90000")], &[96]);
        assert_eq!(parse_rtpmap_h264(&m), Some(96));
    }

    #[test]
    fn rtpmap_ignores_other_codecs_and_unlisted_pts() {
        let m = media_with(
            &[("rtpmap", "97 H265/90000"), ("rtpmap", "98 H264/90000")],
            &[97],
        );
        assert_eq!(parse_rtpmap_h264(&m), None); // 98 not in payload_types
    }

    #[test]
    fn fmtp_parses_mode_and_sprop() {
        // Real-world shape (Axis/GStreamer style). sprop = base64(SPS),base64(PPS).
        // SPS = 67 42 00 1E ..., PPS = 68 CE 38 80 — tiny but type-valid NALUs.
        let m = media_with(
            &[
                ("rtpmap", "96 H264/90000"),
                (
                    "fmtp",
                    "96 packetization-mode=1;profile-level-id=42001e;sprop-parameter-sets=Z0IAHg==,aM44gA==",
                ),
            ],
            &[96],
        );
        let f = H264FmtpParams::parse(&m, 96);
        assert_eq!(f.packetization_mode, 1);
        assert_eq!(f.profile_level_id.as_deref(), Some("42001e"));
        assert_eq!(f.sprop_parameter_sets.len(), 2);
        assert_eq!(f.sprop_parameter_sets[0][0] & 0x1F, 7); // SPS
        assert_eq!(f.sprop_parameter_sets[1][0] & 0x1F, 8); // PPS
    }

    #[test]
    fn fmtp_absent_defaults_mode_0() {
        let m = media_with(&[("rtpmap", "96 H264/90000")], &[96]);
        let f = H264FmtpParams::parse(&m, 96);
        assert_eq!(f.packetization_mode, 0);
        assert!(f.sprop_parameter_sets.is_empty());
    }

    #[test]
    fn fmtp_unparseable_packetization_mode_falls_back_to_0() {
        let m = media_with(
            &[(
                "fmtp",
                "96 packetization-mode=banana;profile-level-id=42001e",
            )],
            &[96],
        );
        let f = H264FmtpParams::parse(&m, 96);
        assert_eq!(f.packetization_mode, 0); // warn + default, not an error
        assert_eq!(f.profile_level_id.as_deref(), Some("42001e")); // later params still parsed
    }

    #[test]
    fn fmtp_out_of_range_packetization_mode_falls_back_to_0() {
        // RFC 6184 §8.2.1: packetization-mode MUST be 0, 1, or 2.
        // Values outside that range are treated the same as unparseable: warn + default 0.
        let m = media_with(
            &[("fmtp", "96 packetization-mode=7;profile-level-id=42001e")],
            &[96],
        );
        let f = H264FmtpParams::parse(&m, 96);
        assert_eq!(f.packetization_mode, 0); // out-of-range → default 0
        assert_eq!(f.profile_level_id.as_deref(), Some("42001e")); // later params still parsed

        // Value 2 is the upper bound of the valid range and must be stored as-is.
        let m2 = media_with(
            &[("fmtp", "96 packetization-mode=2;profile-level-id=42001e")],
            &[96],
        );
        let f2 = H264FmtpParams::parse(&m2, 96);
        assert_eq!(f2.packetization_mode, 2);
    }

    #[test]
    fn fmtp_bad_sprop_entries_are_skipped_not_fatal() {
        let m = media_with(
            &[(
                "fmtp",
                "96 packetization-mode=1;sprop-parameter-sets=!!notb64!!,Z0IAHg==",
            )],
            &[96],
        );
        let f = H264FmtpParams::parse(&m, 96);
        assert_eq!(f.sprop_parameter_sets.len(), 1); // bad entry dropped with a tracing warn
    }

    // --- rtpmap clock-rate validation tests ------------------------------------

    /// Non-90000 clock rate is rejected — RFC 6184 §8.1 mandates 90 kHz.
    #[test]
    fn rtpmap_wrong_clock_rate_rejected() {
        let m = media_with(&[("rtpmap", "96 H264/8000")], &[96]);
        assert_eq!(parse_rtpmap_h264(&m), None);
    }

    /// Case-insensitive encoding name match still requires 90000 clock rate.
    #[test]
    fn rtpmap_case_insensitive_with_correct_clock() {
        let m = media_with(&[("rtpmap", "96 h264/90000")], &[96]);
        assert_eq!(parse_rtpmap_h264(&m), Some(96));
    }

    /// When a media section has both a non-conformant H264/8000 entry and a
    /// valid H264/90000 entry for different PTs, the valid one wins.
    #[test]
    fn rtpmap_bad_clock_does_not_shadow_good_clock() {
        // PT 95 has wrong clock; PT 96 has correct clock — pick_rtpmap must
        // skip 95 and find 96.
        let m = media_with(
            &[("rtpmap", "95 H264/8000"), ("rtpmap", "96 H264/90000")],
            &[95, 96],
        );
        assert_eq!(parse_rtpmap_h264(&m), Some(96));
    }

    #[test]
    fn sprop_unpadded_base64_accepted() {
        // Same SPS/PPS as fmtp_parses_mode_and_sprop with the trailing
        // `=` padding stripped — the common real-camera spelling.
        let m = media_with(
            &[
                ("rtpmap", "96 H264/90000"),
                (
                    "fmtp",
                    "96 packetization-mode=1;sprop-parameter-sets=Z0IAHg,aM44gA",
                ),
            ],
            &[96],
        );
        let f = H264FmtpParams::parse(&m, 96);
        assert_eq!(
            f.sprop_parameter_sets.len(),
            2,
            "unpadded sprop entries must decode"
        );
        assert_eq!(f.sprop_parameter_sets[0], vec![0x67, 0x42, 0x00, 0x1E]);
        assert_eq!(f.sprop_parameter_sets[1], vec![0x68, 0xCE, 0x38, 0x80]);
    }

    #[test]
    fn sprop_invalid_chars_still_skipped() {
        let m = media_with(
            &[
                ("rtpmap", "96 H264/90000"),
                ("fmtp", "96 sprop-parameter-sets=Z0IAHg,@@notbase64@@"),
            ],
            &[96],
        );
        let f = H264FmtpParams::parse(&m, 96);
        assert_eq!(
            f.sprop_parameter_sets.len(),
            1,
            "invalid entry skipped, valid unpadded entry kept"
        );
    }
}
