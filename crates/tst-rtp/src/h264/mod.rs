//! RFC 6184 H.264 RTP payload format support.
//!
//! This module provides SDP attribute parsing and (in later tasks) the
//! depacketizer state machine for reassembling H.264 NALUs from RTP packets.
//!
//! # SDP parsing
//!
//! Use [`parse_rtpmap_h264`] to discover the dynamic payload type assigned
//! to H.264 from the `a=rtpmap` lines, then [`H264FmtpParams::parse`] to
//! extract packetization mode, out-of-band parameter sets (SPS/PPS), and
//! profile information from `a=fmtp`.

pub mod fmtp;

pub use fmtp::{H264FmtpParams, parse_rtpmap_h264};
