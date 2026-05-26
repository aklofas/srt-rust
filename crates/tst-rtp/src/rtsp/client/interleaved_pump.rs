//! Client-side interleaved producer thread — reads `$`-prefixed binary
//! frames + RTSP responses off the client's `Stream` read half
//! (post-PLAY when `transport_kind == TcpInterleaved`), demuxes by
//! channel, and routes:
//!
//! - Binary on `rtp_channel`: strip the 12-byte RTP header, push the
//!   payload to `data_tx` (the `mpsc::Sender<Bytes>` paired with the
//!   `mpsc::Receiver<Bytes>` that `RtpRecvTransport::from_mpsc_placeholder`
//!   consumes).
//! - Binary on `rtcp_channel`: push payload to `rtcp_tx` (the RTCP
//!   ingest sink).
//! - RTSP responses (`CRLFCRLF`-framed + `Content-Length` body): push to
//!   `ctrl_tx`, which the main thread polls instead of reading the
//!   `Stream` directly once interleaved-PLAY mode is active.
//!
//! Mirror of the server-side pump
//! ([`crate::rtsp::server::interleaved_pump`]). Closes Phase 2 deferred
//! fix 1 (client side) — see
//! `[[feedback-wire-primitives-at-call-site-as-explicit-task]]`.
//!
//! # Wire-up status
//!
//! This task ships only the `spawn_client_pump` primitive function +
//! its unit tests. The actual wire-up into
//! [`crate::rtsp::client::RtspClient::play`] (spawning the pump after
//! PLAY succeeds, plumbing `data_tx` / `ctrl_rx` through
//! `RtspSession::into_recv_transport`) is deferred to a Wave H
//! follow-up. `RtpRecvTransport::from_mpsc_placeholder` still receives
//! a never-fed channel from `RtspSession::into_recv_transport` until
//! that wire-up lands.

use std::io::Read;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread::JoinHandle;

use bytes::Bytes;

use crate::packet::RTP_HEADER_LEN;

/// Stats observable from outside the pump thread.
#[derive(Default)]
#[allow(dead_code)]
pub(crate) struct PumpStats {
    /// Count of complete RTSP responses (header + body) routed to
    /// `ctrl_tx`.
    pub(crate) rtsp_messages_received: AtomicU64,
    /// Count of `$<rtp_channel>` binary frames whose RTP header was
    /// stripped + payload pushed to `data_tx`.
    pub(crate) rtp_frames_received: AtomicU64,
    /// Count of `$<rtcp_channel>` binary frames whose payload was pushed
    /// to `rtcp_tx`.
    pub(crate) rtcp_frames_received: AtomicU64,
    /// Count of frames dropped because they were on an unknown channel,
    /// too small to contain an RTP header, or had a non-UTF8 RTSP
    /// header line.
    pub(crate) malformed_frames: AtomicU64,
}

/// SETUP-allocated channel pair. The SETUP handler assigns these (the
/// client requests `interleaved=N-M` in its Transport header; the server
/// echoes or reassigns).
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub(crate) struct InterleavedChannels {
    /// Channel number for RTP data frames.
    pub(crate) rtp: u8,
    /// Channel number for RTCP frames (conventionally `rtp + 1`).
    pub(crate) rtcp: u8,
}

/// Spawn the sync interleaved pump thread.
///
/// `reader` is any sync [`Read`] implementor. For plain TCP the caller
/// passes a [`std::net::TcpStream::try_clone`]'d half. For TLS the
/// caller passes a lock-and-read shim over the rustls session (Task 21
/// lands the adapter).
///
/// - `data_tx`: where stripped RTP payloads go (paired with
///   `RtpRecvTransport::from_mpsc_placeholder`).
/// - `rtcp_tx`: where RTCP `$<rtcp_channel>` frame payloads go.
/// - `ctrl_tx`: where RTSP response bytes go — main thread polls.
/// - `channels`: SETUP-allocated channel pair.
/// - `cancel`: flipped by `RtspClient::drop` or
///   [`crate::rtsp::client::RtspCancelHandle::cancel`].
/// - `stats`: observable counters.
///
/// The pump exits cleanly when:
/// - `reader.read()` returns `Ok(0)` (EOF).
/// - `cancel` is set and the next `read()` returns
///   [`std::io::ErrorKind::TimedOut`] / [`std::io::ErrorKind::WouldBlock`].
/// - `data_tx.send()` errors (the receiver was dropped).
/// - `reader.read()` returns a non-timeout error (logged at WARN).
#[allow(dead_code, clippy::too_many_arguments)]
pub(crate) fn spawn_client_pump<R: Read + Send + 'static>(
    mut reader: R,
    data_tx: mpsc::Sender<Bytes>,
    rtcp_tx: mpsc::Sender<Bytes>,
    ctrl_tx: mpsc::Sender<Bytes>,
    channels: InterleavedChannels,
    cancel: Arc<AtomicBool>,
    stats: Arc<PumpStats>,
) -> JoinHandle<()> {
    std::thread::Builder::new()
        .name("rtsp-interleaved-pump".to_string())
        .spawn(move || {
            let mut buf: Vec<u8> = Vec::with_capacity(16384);
            let mut chunk = [0u8; 4096];
            loop {
                if cancel.load(Ordering::Relaxed) {
                    return;
                }
                let n = match reader.read(&mut chunk) {
                    Ok(0) => return, // Clean EOF.
                    Ok(n) => n,
                    Err(e)
                        if e.kind() == std::io::ErrorKind::TimedOut
                            || e.kind() == std::io::ErrorKind::WouldBlock =>
                    {
                        // The TcpStream is configured with a 100 ms read
                        // timeout in RtspClient::connect_with — loop back
                        // to recheck the cancel flag.
                        continue;
                    }
                    Err(e) => {
                        tracing::warn!(
                            target: "tst_rtp::client::pump",
                            error = %e,
                            "TCP read failed; pump exiting"
                        );
                        return;
                    }
                };
                buf.extend_from_slice(&chunk[..n]);

                // Parse as many complete frames as possible.
                loop {
                    if buf.is_empty() {
                        break;
                    }
                    if buf[0] == b'$' {
                        // Binary interleaved frame: `$<channel><len_be16><payload>`.
                        if buf.len() < 4 {
                            break; // Need more bytes to read the header.
                        }
                        let channel = buf[1];
                        let length = u16::from_be_bytes([buf[2], buf[3]]) as usize;
                        if buf.len() < 4 + length {
                            break; // Need more bytes to read the body.
                        }
                        let payload_bytes = &buf[4..4 + length];
                        if channel == channels.rtp {
                            stats.rtp_frames_received.fetch_add(1, Ordering::Relaxed);
                            // Strip the 12-byte RTP header. If the frame
                            // is too small to contain a header, drop and
                            // counter-tick.
                            if payload_bytes.len() < RTP_HEADER_LEN {
                                stats.malformed_frames.fetch_add(1, Ordering::Relaxed);
                            } else {
                                let ts_payload =
                                    Bytes::copy_from_slice(&payload_bytes[RTP_HEADER_LEN..]);
                                if data_tx.send(ts_payload).is_err() {
                                    // Receiver dropped — pump exits.
                                    return;
                                }
                            }
                        } else if channel == channels.rtcp {
                            stats.rtcp_frames_received.fetch_add(1, Ordering::Relaxed);
                            let _ = rtcp_tx.send(Bytes::copy_from_slice(payload_bytes));
                        } else {
                            stats.malformed_frames.fetch_add(1, Ordering::Relaxed);
                            tracing::warn!(
                                target: "tst_rtp::client::pump",
                                channel = channel,
                                "frame on unknown channel; dropping"
                            );
                        }
                        buf.drain(..4 + length);
                    } else {
                        // RTSP response. Frame on `CRLFCRLF` + body of
                        // `Content-Length` bytes.
                        let end = match buf.windows(4).position(|w| w == b"\r\n\r\n") {
                            Some(e) => e,
                            None => break, // Need more bytes.
                        };
                        let header_text = match std::str::from_utf8(&buf[..end]) {
                            Ok(s) => s,
                            Err(_) => {
                                stats.malformed_frames.fetch_add(1, Ordering::Relaxed);
                                buf.drain(..end + 4);
                                continue;
                            }
                        };
                        let content_length: usize = header_text
                            .lines()
                            .find_map(|line| {
                                let lower = line.to_ascii_lowercase();
                                lower
                                    .strip_prefix("content-length:")
                                    .and_then(|v| v.trim().parse().ok())
                            })
                            .unwrap_or(0);
                        let msg_end = end + 4 + content_length;
                        if buf.len() < msg_end {
                            break; // Need more bytes for the body.
                        }
                        stats.rtsp_messages_received.fetch_add(1, Ordering::Relaxed);
                        let msg = Bytes::copy_from_slice(&buf[..msg_end]);
                        if ctrl_tx.send(msg).is_err() {
                            // Main thread isn't reading — silently drop;
                            // the cancel flag will eventually flip on
                            // RtspClient::drop and we'll exit.
                            tracing::warn!(
                                target: "tst_rtp::client::pump",
                                "ctrl_tx closed; pump losing RTSP responses"
                            );
                        }
                        buf.drain(..msg_end);
                    }
                }
            }
        })
        .expect("failed to spawn rtsp-interleaved-pump thread")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[allow(clippy::type_complexity)]
    fn make_args() -> (
        mpsc::Sender<Bytes>,
        mpsc::Receiver<Bytes>,
        mpsc::Sender<Bytes>,
        mpsc::Receiver<Bytes>,
        mpsc::Sender<Bytes>,
        mpsc::Receiver<Bytes>,
        Arc<AtomicBool>,
        Arc<PumpStats>,
    ) {
        let (dt, dr) = mpsc::channel();
        let (rt, rr) = mpsc::channel();
        let (ct, cr) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let stats = Arc::new(PumpStats::default());
        (dt, dr, rt, rr, ct, cr, cancel, stats)
    }

    /// Feed an RTP frame (channel=0, 12-byte header + 8-byte payload).
    /// Pump should strip the header and push the 8 bytes onto data_rx.
    #[test]
    fn rtp_frame_stripped_and_delivered() {
        let mut raw = vec![b'$', 0u8, 0x00, 20];
        raw.extend_from_slice(&[0u8; RTP_HEADER_LEN]); // RTP header (12 zeros).
        raw.extend_from_slice(b"PAYLOAD!"); // 8 bytes.
        let (dt, dr, rt, _rr, ct, _cr, cancel, stats) = make_args();
        let handle = spawn_client_pump(
            Cursor::new(raw),
            dt,
            rt,
            ct,
            InterleavedChannels { rtp: 0, rtcp: 1 },
            cancel.clone(),
            stats.clone(),
        );
        // Pump should deliver one payload, then EOF and exit.
        let payload = dr.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
        assert_eq!(payload.as_ref(), b"PAYLOAD!");
        let _ = handle.join();
        assert_eq!(stats.rtp_frames_received.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn rtcp_frame_routed_to_rtcp_rx() {
        let mut raw = vec![b'$', 1u8, 0x00, 4];
        raw.extend_from_slice(b"\xDE\xAD\xBE\xEF");
        let (dt, _dr, rt, rr, ct, _cr, cancel, stats) = make_args();
        let handle = spawn_client_pump(
            Cursor::new(raw),
            dt,
            rt,
            ct,
            InterleavedChannels { rtp: 0, rtcp: 1 },
            cancel.clone(),
            stats.clone(),
        );
        let payload = rr.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
        assert_eq!(payload.as_ref(), &[0xDE, 0xAD, 0xBE, 0xEF]);
        let _ = handle.join();
        assert_eq!(stats.rtcp_frames_received.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn rtsp_response_routed_to_ctrl_rx() {
        let raw = b"RTSP/1.0 200 OK\r\nCSeq: 1\r\n\r\n".to_vec();
        let (dt, _dr, rt, _rr, ct, cr, cancel, stats) = make_args();
        let handle = spawn_client_pump(
            Cursor::new(raw),
            dt,
            rt,
            ct,
            InterleavedChannels { rtp: 0, rtcp: 1 },
            cancel.clone(),
            stats.clone(),
        );
        let msg = cr.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
        let text = std::str::from_utf8(&msg).unwrap();
        assert!(text.starts_with("RTSP/1.0 200 OK"));
        let _ = handle.join();
        assert_eq!(stats.rtsp_messages_received.load(Ordering::Relaxed), 1);
    }

    /// RTSP response with a `Content-Length: N` body — the pump must
    /// wait for the body bytes before considering the message complete.
    #[test]
    fn rtsp_response_with_body_routed_to_ctrl_rx() {
        let raw = b"RTSP/1.0 200 OK\r\nCSeq: 2\r\nContent-Length: 5\r\n\r\nHELLO".to_vec();
        let (dt, _dr, rt, _rr, ct, cr, cancel, stats) = make_args();
        let handle = spawn_client_pump(
            Cursor::new(raw),
            dt,
            rt,
            ct,
            InterleavedChannels { rtp: 0, rtcp: 1 },
            cancel.clone(),
            stats.clone(),
        );
        let msg = cr.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
        assert!(msg.ends_with(b"\r\n\r\nHELLO"));
        let _ = handle.join();
        assert_eq!(stats.rtsp_messages_received.load(Ordering::Relaxed), 1);
    }

    /// A frame on an unknown channel (not rtp, not rtcp) should be
    /// counter-ticked + dropped, but not crash the pump.
    #[test]
    fn unknown_channel_frame_counted_and_dropped() {
        let raw = vec![b'$', 7u8, 0x00, 2, 0xAB, 0xCD];
        let (dt, _dr, rt, _rr, ct, _cr, cancel, stats) = make_args();
        let handle = spawn_client_pump(
            Cursor::new(raw),
            dt,
            rt,
            ct,
            InterleavedChannels { rtp: 0, rtcp: 1 },
            cancel.clone(),
            stats.clone(),
        );
        let _ = handle.join();
        assert_eq!(stats.malformed_frames.load(Ordering::Relaxed), 1);
        assert_eq!(stats.rtp_frames_received.load(Ordering::Relaxed), 0);
        assert_eq!(stats.rtcp_frames_received.load(Ordering::Relaxed), 0);
    }

    /// An RTP frame whose body is smaller than the 12-byte RTP header
    /// should be counter-ticked and dropped (not panic on slice).
    #[test]
    fn undersized_rtp_frame_counted_and_dropped() {
        let raw = vec![b'$', 0u8, 0x00, 4, 0x00, 0x00, 0x00, 0x00];
        let (dt, _dr, rt, _rr, ct, _cr, cancel, stats) = make_args();
        let handle = spawn_client_pump(
            Cursor::new(raw),
            dt,
            rt,
            ct,
            InterleavedChannels { rtp: 0, rtcp: 1 },
            cancel.clone(),
            stats.clone(),
        );
        let _ = handle.join();
        assert_eq!(stats.rtp_frames_received.load(Ordering::Relaxed), 1);
        assert_eq!(stats.malformed_frames.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn cancel_flag_exits_pump() {
        // Use an empty Cursor — gives EOF immediately. We can't easily
        // simulate a never-EOF stream with Cursor, so we set cancel
        // before the pump starts; the test verifies pump joins cleanly
        // when cancel is set.
        let raw = Vec::<u8>::new();
        let (dt, _dr, rt, _rr, ct, _cr, cancel, stats) = make_args();
        cancel.store(true, Ordering::Relaxed);
        let handle = spawn_client_pump(
            Cursor::new(raw),
            dt,
            rt,
            ct,
            InterleavedChannels { rtp: 0, rtcp: 1 },
            cancel.clone(),
            stats.clone(),
        );
        let _ = handle.join();
    }
}
