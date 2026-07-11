//! [`H264Receiver`] — blocking I/O shell for the RFC 6184 H.264 depacketizer.
//!
//! Wraps a UDP socket or mpsc channel (TCP-interleaved bridge), polls
//! `Source::recv_raw` with cancel-poll cadence, decodes the RTP header,
//! filters by payload type, and feeds each packet into
//! [`H264Depacketizer`], surfacing reassembled [`H264Au`]s via
//! [`H264Receiver::recv_au`].
//!
//! # URL form
//!
//! ```text
//! rtp://127.0.0.1:0?pt=96
//! ```
//!
//! `?pt=` is **required** (range 1..=127, 33 rejected — use the MP2T
//! receiver for MPEG-TS). The MP2T constructors (`RtpTransport::connect*`
//! / `RtpRecvTransport::listen*`) conversely reject URLs carrying `?pt=`.
//!
//! # Blocking / threading model
//!
//! [`H264Receiver::recv_au`] blocks the calling thread. Callers that need
//! concurrent mux/demux should run the receiver on a dedicated thread and
//! channel AUs out. A single thread must own the receiver — concurrent
//! calls to `recv_au` are not supported.
//!
//! # RTCP
//!
//! RTCP is **not** implemented on this path (v1 decision). No RTCP socket
//! is bound; no RR/SR packets are sent or received. This is a recorded
//! deferral: see `docs/project/deferred-features.md`.
//!
//! # EOS contract
//!
//! `recv_au` returns `Ok(None)` in two situations:
//! - `close()` was called (cancel flag set), or
//! - The underlying source reports a clean disconnect (mpsc pump dropped
//!   its `Sender`, indicating the RTSP teardown completed).
//!
//! Any other hard I/O error surfaces as `Err(TransportError::Broken {..})`.

use std::net::{SocketAddr, UdpSocket};
use std::sync::Arc;

use tst_core::net::udp_socket::CANCEL_POLL_INTERVAL;
use tst_core::transport::TransportError;

use crate::cancel::RtpCancelHandle;
use crate::h264::depacketizer::{H264Au, H264Depacketizer, H264DepayConfig, H264DepayStats};
use crate::packet::RtpHeader;
use crate::transport::{ConnectError, RECV_SCRATCH_LEN, RtpStats, Source};
use crate::url::RtpUrl;

/// Blocking H.264-over-RTP receive shell.
///
/// # Constructing
///
/// - [`Self::listen`] — parse `rtp://host:port?pt=N`, bind, and return a
///   ready receiver. `?pt=` is required; value 33 is rejected.
/// - [`Self::listen_with`] — same, but from an already-parsed [`RtpUrl`].
/// - `from_udp_socket_with` (`pub(crate)`) — wrap an already-bound socket;
///   used by the RTSP session bridge (Task 11).
/// - `from_mpsc_with` (`pub(crate)`) — wrap a TCP-interleaved mpsc channel;
///   used by the RTSP session bridge (Task 11).
///
/// # recv_au loop contract
///
/// 1. Drain `depay.next_au()` first — the depacketizer may have queued
///    AUs from a prior `feed` call.
/// 2. If the EOS flag is set, return `Ok(None)`.
/// 3. Call `source.recv_raw(scratch, cancel)`:
///    - `Ok(n)` — tick counters, decode RTP header (malformed → tick +
///      continue), check payload type (wrong PT → tick + continue), feed
///      depacketizer, go to step 1.
///    - `Err(ExplicitClose)` — set EOS, flush depacketizer, return
///      flushed AU if any else `Ok(None)`.
///    - `Err(Broken)` where the message is `"interleaved pump bridge
///      disconnected"` — same EOS path (clean RTSP teardown).
///    - `Err(Broken)` otherwise — clear source, propagate.
///
/// # RTCP
///
/// RTCP is not implemented on this path (v1 decision). No RTCP companion
/// socket is bound. See `docs/project/deferred-features.md`.
pub struct H264Receiver {
    source: Option<Source>,
    scratch: Vec<u8>,
    cancel: Arc<RtpCancelHandle>,
    depay: H264Depacketizer,
    pt: u8,
    bytes_received: u64,
    packets_received: u64,
    malformed_packets: u64,
    local_addr: Option<SocketAddr>,
    eos: bool,
}

impl H264Receiver {
    /// Bind to `url`'s host:port and return a ready receiver.
    ///
    /// `url` must carry `?pt=N` (1..=127, 33 rejected). The default
    /// [`H264DepayConfig`] is constructed from the URL's `pt` value.
    ///
    /// # Errors
    ///
    /// Returns [`ConnectError::MissingPayloadTypeParam`] when `?pt=` is
    /// absent, [`ConnectError::Url`] on parse failure, or
    /// [`ConnectError::Io`] on bind failure.
    pub fn listen(url: &str) -> Result<Self, ConnectError> {
        let parsed = RtpUrl::parse(url).map_err(ConnectError::Url)?;
        Self::listen_with(&parsed, H264DepayConfig::default())
    }

    /// Bind using an already-parsed URL with an explicit depacketizer
    /// config. `url.pt` must be `Some(_)`.
    ///
    /// # Errors
    ///
    /// Returns [`ConnectError::MissingPayloadTypeParam`] when `url.pt` is
    /// `None`, or [`ConnectError::Io`] on bind failure.
    pub fn listen_with(url: &RtpUrl, mut config: H264DepayConfig) -> Result<Self, ConnectError> {
        let pt = url.pt.ok_or(ConnectError::MissingPayloadTypeParam)?;
        config.payload_type = pt;
        let ip: std::net::IpAddr = url.host.parse().map_err(|e: std::net::AddrParseError| {
            ConnectError::HostNotLiteral {
                host: url.host.clone(),
                detail: e.to_string(),
            }
        })?;
        let local = SocketAddr::new(ip, url.port);
        let sock = UdpSocket::bind(local).map_err(ConnectError::Io)?;
        Self::from_udp_socket_with(sock, config)
    }

    /// Wrap an already-bound socket. Sets the cancel-poll read timeout to
    /// match the rest of the recv-side machinery.
    ///
    /// Used by Task 11's RTSP session bridge.
    pub(crate) fn from_udp_socket_with(
        sock: UdpSocket,
        config: H264DepayConfig,
    ) -> Result<Self, ConnectError> {
        sock.set_read_timeout(Some(CANCEL_POLL_INTERVAL))
            .map_err(ConnectError::Io)?;
        let local_addr = sock.local_addr().ok();
        let pt = config.payload_type;
        Ok(Self {
            source: Some(Source::Udp(sock)),
            scratch: vec![0u8; RECV_SCRATCH_LEN],
            cancel: RtpCancelHandle::new(),
            depay: H264Depacketizer::new(config),
            pt,
            bytes_received: 0,
            packets_received: 0,
            malformed_packets: 0,
            local_addr,
            eos: false,
        })
    }

    /// Wrap an mpsc channel fed by the RTSP client's TCP-interleaved pump.
    ///
    /// The producer (pump thread) pushes **whole RTP packets** (header
    /// intact). [`recv_au`](Self::recv_au) decodes the header, enforces the
    /// configured PT, strips CSRC/extension/padding, and feeds the
    /// depacketizer — mirroring the UDP arm end-to-end.
    ///
    /// Used by Task 11's RTSP session bridge.
    #[allow(dead_code)]
    pub(crate) fn from_mpsc_with(
        rx: std::sync::mpsc::Receiver<bytes::Bytes>,
        config: H264DepayConfig,
    ) -> Self {
        let pt = config.payload_type;
        Self {
            source: Some(Source::Mpsc(rx)),
            scratch: vec![0u8; RECV_SCRATCH_LEN],
            cancel: RtpCancelHandle::new(),
            depay: H264Depacketizer::new(config),
            pt,
            bytes_received: 0,
            packets_received: 0,
            malformed_packets: 0,
            local_addr: None,
            eos: false,
        }
    }

    /// Receive the next reassembled H.264 Access Unit.
    ///
    /// Returns `Ok(Some(au))` when a complete AU is available,
    /// `Ok(None)` at EOS (cancel/close or clean RTSP teardown), and
    /// `Err(TransportError::Broken {..})` on a hard I/O error.
    ///
    /// Blocks the calling thread. See the [struct-level doc](Self) for the
    /// full loop contract.
    pub fn recv_au(&mut self) -> Result<Option<H264Au>, TransportError> {
        loop {
            // ── Step 1: drain the depacketizer's ready queue ──────────────
            if let Some(au) = self.depay.next_au() {
                return Ok(Some(au));
            }

            // ── Step 2: check EOS ─────────────────────────────────────────
            if self.eos {
                return Ok(None);
            }

            // ── Step 3: recv_raw ──────────────────────────────────────────
            let source = match self.source.as_ref() {
                Some(s) => s,
                None => return Err(TransportError::Closed),
            };
            let raw_result = source.recv_raw(&mut self.scratch, &self.cancel);
            let n = match raw_result {
                Ok(n) => n,
                Err(TransportError::ExplicitClose) => {
                    // Cancel / close — drain depacketizer then EOS.
                    self.eos = true;
                    return Ok(self.depay.flush());
                }
                Err(TransportError::Broken { ref msg, .. })
                    if msg == "interleaved pump bridge disconnected" =>
                {
                    // Clean RTSP teardown — same EOS path.
                    self.eos = true;
                    return Ok(self.depay.flush());
                }
                Err(e @ TransportError::Broken { .. }) => {
                    self.source = None;
                    return Err(e);
                }
                Err(e) => return Err(e),
            };

            // ── Count at wire-level, before validation ────────────────────
            self.bytes_received = self.bytes_received.saturating_add(n as u64);
            self.packets_received = self.packets_received.saturating_add(1);

            // ── Decode RTP header ─────────────────────────────────────────
            let parsed = match RtpHeader::decode(&self.scratch[..n]) {
                Ok(p) => p,
                Err(_) => {
                    self.malformed_packets = self.malformed_packets.saturating_add(1);
                    continue;
                }
            };

            // ── PT filter ────────────────────────────────────────────────
            if parsed.header.payload_type != self.pt {
                self.malformed_packets = self.malformed_packets.saturating_add(1);
                continue;
            }

            // ── Feed depacketizer ─────────────────────────────────────────
            let payload = &self.scratch[parsed.payload_offset..parsed.payload_end];
            self.depay.feed(&parsed.header, payload);
            // Go back to step 1 to drain next_au.
        }
    }

    /// Statistics from the RFC 6184 depacketizer (AU counts, seq gaps,
    /// parameter-set updates, etc.).
    pub fn depay_stats(&self) -> H264DepayStats {
        self.depay.stats()
    }

    /// RTP-protocol-level statistics (malformed packet counter).
    pub fn rtp_stats(&self) -> RtpStats {
        RtpStats {
            malformed_packets: self.malformed_packets,
        }
    }

    /// The local address the UDP socket is bound to, or `None` for the
    /// mpsc (TCP-interleaved) path.
    ///
    /// Tests use this to discover the ephemeral port assigned by the kernel
    /// when `port=0` was specified in the URL.
    pub fn local_addr(&self) -> Option<SocketAddr> {
        self.local_addr
    }

    /// Return a clone of the cancel handle. Setting the flag causes the
    /// next [`recv_au`](Self::recv_au) poll to return EOS.
    ///
    /// The returned value is an `Arc<RtpCancelHandle>`-compatible clone —
    /// callers hold a shared reference and set it from any thread.
    pub fn cancel_handle(&self) -> Arc<RtpCancelHandle> {
        self.cancel.clone()
    }

    /// Cancel any in-progress [`recv_au`](Self::recv_au) and drop the
    /// underlying source. Idempotent.
    pub fn close(&mut self) {
        self.cancel.cancel();
        self.source = None;
        self.eos = true;
    }
}

impl Drop for H264Receiver {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::h264::depacketizer::H264DepayConfig;

    /// Minimal UDP loopback: bind an `H264Receiver` on an ephemeral port,
    /// send one well-formed IDR NALU packet (single-NALU mode, M=1, PT=96),
    /// verify the reassembled AU has the expected Annex B framing and the
    /// `key_frame` flag set.
    ///
    /// Packet layout (RFC 3550 §5.1 + RFC 6184 §5.6):
    ///   Byte 0:  0x80  (V=2, P=0, X=0, CC=0)
    ///   Byte 1:  0x80 | 96  (M=1, PT=96)
    ///   Bytes 2..4: seq=1
    ///   Bytes 4..8: ts=0x00002328 (9000 ticks)
    ///   Bytes 8..12: ssrc=9
    ///   Byte 12: 0x65 (NALU type 5 = IDR)
    ///   Bytes 13..: payload bytes
    #[test]
    fn h264_receiver_udp_loopback_single_au() {
        let mut rx = H264Receiver::listen("rtp://127.0.0.1:0?pt=96").unwrap();
        let dst = rx.local_addr().unwrap();
        let tx = UdpSocket::bind("127.0.0.1:0").unwrap();
        // Hand-built per RFC 3550 §5.1: V=2, M=1, PT=96, seq=1, ts=9000,
        // ssrc=9; payload = type-5 IDR bytes 0x65 0xAB 0xCD.
        let pkt = [
            0x80u8,
            0x80 | 96,
            0,
            1,
            0,
            0,
            0x23,
            0x28,
            0,
            0,
            0,
            9,
            0x65,
            0xAB,
            0xCD,
        ];
        tx.send_to(&pkt, dst).unwrap();
        let au = rx.recv_au().unwrap().expect("AU expected");
        assert_eq!(au.annexb, [0, 0, 0, 1, 0x65, 0xAB, 0xCD]);
        assert!(au.key_frame);
        rx.close();
    }

    /// `listen` without `?pt=` must return `MissingPayloadTypeParam`.
    #[test]
    fn listen_without_pt_errors() {
        let result = H264Receiver::listen("rtp://127.0.0.1:0").map(|_| ());
        let err = result.expect_err("listen without ?pt= must fail");
        assert!(
            matches!(err, ConnectError::MissingPayloadTypeParam),
            "expected MissingPayloadTypeParam, got {err:?}"
        );
    }

    /// `from_mpsc_with` + send a single IDR packet via the channel.
    #[test]
    fn h264_receiver_mpsc_single_au() {
        use crate::packet::RTP_HEADER_LEN;
        use bytes::Bytes;

        let (tx, rx) = std::sync::mpsc::channel::<Bytes>();
        let config = H264DepayConfig {
            payload_type: 96,
            ..H264DepayConfig::default()
        };
        let mut receiver = H264Receiver::from_mpsc_with(rx, config);

        // Build a whole RTP packet (header + payload).
        let mut pkt = vec![0u8; RTP_HEADER_LEN];
        let mut hdr = crate::packet::RtpHeader::new(1, 9000, 0xABCD);
        hdr.marker = true;
        hdr.payload_type = 96;
        hdr.encode_into(&mut pkt);
        pkt.extend_from_slice(&[0x65u8, 0xEF]); // IDR slice bytes
        tx.send(Bytes::from(pkt)).unwrap();

        let au = receiver.recv_au().unwrap().expect("AU expected");
        assert_eq!(au.annexb, [0, 0, 0, 1, 0x65, 0xEF]);
        assert!(au.key_frame);
    }

    /// EOS via mpsc disconnect — dropping the sender should yield `Ok(None)`.
    #[test]
    fn mpsc_disconnect_yields_eos() {
        use bytes::Bytes;
        let (tx, rx) = std::sync::mpsc::channel::<Bytes>();
        let config = H264DepayConfig {
            payload_type: 96,
            ..H264DepayConfig::default()
        };
        let mut receiver = H264Receiver::from_mpsc_with(rx, config);
        drop(tx); // causes Disconnected on recv_raw
        let result = receiver.recv_au().unwrap();
        assert!(result.is_none(), "expected Ok(None) at EOS, got {result:?}");
    }
}
