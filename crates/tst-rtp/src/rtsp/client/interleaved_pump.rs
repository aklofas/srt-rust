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
//! As of Phase 3 Wave H Task 4 (2026-05-26) the pump is wired into the
//! TCP-interleaved SETUP path: `RtspClient::activate_interleaved_pump`
//! (crate-private) spawns the pump as soon as a TCP-interleaved SETUP
//! succeeds (so the pump is draining the wire before PLAY is sent), and
//! [`RtspSession::into_recv_transport`](crate::rtsp::client::session::RtspSession::into_recv_transport)
//! consumes the data-side `mpsc::Receiver<Bytes>` plumbed through
//! `RtpRecvTransport::from_mpsc_placeholder` (crate-private). Once the
//! pump is active, subsequent RTSP request/response exchanges
//! (`RtspClient::send_and_read`, crate-private) write under the stream
//! mutex but read the response from the pump's `ctrl_rx` (matched by
//! CSeq), so reads don't race against the pump.

use std::io::Read;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread::JoinHandle;

use bytes::Bytes;

use crate::packet::RTP_HEADER_LEN;
use crate::rtsp::client::Stream;
use crate::rtsp::message::{MAX_RTSP_MESSAGE_BYTES, content_length_from_header_text};

/// Stats observable from outside the pump thread.
#[derive(Debug, Default)]
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
    /// Count of read attempts the pump actually performed (i.e. loop
    /// iterations where it was not yielding the lock to a control-path
    /// writer). Used by the back-off regression test to assert the pump
    /// stops reading while `write_gate` is set.
    pub(crate) reads_attempted: AtomicU64,
}

/// SETUP-allocated channel pair. The SETUP handler assigns these (the
/// client requests `interleaved=N-M` in its Transport header; the server
/// echoes or reassigns).
#[derive(Debug, Clone, Copy)]
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
/// - `write_gate`: set by a control-path writer
///   ([`crate::rtsp::client::RtspClient::send_and_read_via_pump_with_deadline`])
///   right before it acquires the stream mutex to write a request. While
///   it is set, the pump skips its read (and so does not re-acquire the
///   mutex), letting the writer in promptly. Without this the pump — which
///   holds the mutex across each blocking ~100 ms read — monopolizes the
///   lock and starves in-session writes on contended runners.
/// - `stats`: observable counters.
///
/// The pump exits cleanly when:
/// - `reader.read()` returns `Ok(0)` (EOF).
/// - `cancel` is set and the next `read()` returns
///   [`std::io::ErrorKind::TimedOut`] / [`std::io::ErrorKind::WouldBlock`].
/// - `data_tx.send()` errors (the receiver was dropped).
/// - `reader.read()` returns a non-timeout error (logged at WARN).
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_client_pump<R: Read + Send + 'static>(
    mut reader: R,
    data_tx: mpsc::Sender<Bytes>,
    rtcp_tx: mpsc::Sender<Bytes>,
    ctrl_tx: mpsc::Sender<Bytes>,
    channels: InterleavedChannels,
    cancel: Arc<AtomicBool>,
    write_gate: Arc<AtomicBool>,
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
                // Yield the stream lock to a control-path writer that is
                // about to (or is) waiting for it. We hold the mutex across
                // the blocking read below, so re-acquiring it every cycle
                // would starve the writer; skipping the read for one cycle
                // (~1 ms) lets it acquire promptly. Bounds an in-session
                // PLAY/PAUSE write to at most one in-flight read cycle.
                if write_gate.load(Ordering::Relaxed) {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                    continue;
                }
                stats.reads_attempted.fetch_add(1, Ordering::Relaxed);
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

                // Buffer cap: mirror the server pump. If accumulated bytes
                // exceed the shared RTSP message cap without yielding complete
                // frames, the peer is malformed or adversarial — e.g. a binary
                // frame header or an RTSP response that never completes (no
                // CRLFCRLF terminator, or an oversized declared body that never
                // arrives). There is no 413 to send mid-stream on an
                // interleaved control channel, and continuing would grow `buf`
                // unbounded, so close the pump. (Closes the B1-flagged gap: the
                // client pump header buffer was previously uncapped.)
                if buf.len() > MAX_RTSP_MESSAGE_BYTES {
                    tracing::warn!(
                        target: "tst_rtp::client::pump",
                        buf_len = buf.len(),
                        "pump buffer exceeded {} bytes; closing",
                        MAX_RTSP_MESSAGE_BYTES
                    );
                    stats.malformed_frames.fetch_add(1, Ordering::Relaxed);
                    return;
                }

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
                        // Strict Content-Length: unparseable, oversized
                        // (> MAX_RTSP_BODY_BYTES), or duplicate are all hostile.
                        // Silently coercing to 0 would desync framing, and an
                        // uncapped length would let a peer drive unbounded
                        // buffering — close the pump instead.
                        let content_length = match content_length_from_header_text(header_text) {
                            Ok(n) => n,
                            Err(detail) => {
                                tracing::warn!(
                                    target: "tst_rtp::client::pump",
                                    detail,
                                    "malformed RTSP Content-Length; pump exiting"
                                );
                                stats.malformed_frames.fetch_add(1, Ordering::Relaxed);
                                return;
                            }
                        };
                        // content_length <= MAX_RTSP_BODY_BYTES and end < buf.len(),
                        // so this can't overflow; checked_add for defense in depth.
                        let msg_end = match end
                            .checked_add(4)
                            .and_then(|e| e.checked_add(content_length))
                        {
                            Some(m) => m,
                            None => {
                                stats.malformed_frames.fetch_add(1, Ordering::Relaxed);
                                return;
                            }
                        };
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

/// `Read` adapter that locks `Arc<Mutex<Stream>>` per call.
///
/// The pump runs in a background thread and shares the control TCP /
/// TLS stream with the main thread (request writes) and the keepalive
/// thread (OPTIONS writes). To avoid `try_clone` — which doesn't work
/// for rustls `ClientConnection` — we lock the same `Arc<Mutex<Stream>>`
/// once per `read()` call. Each call:
///
/// 1. Acquires the mutex.
/// 2. Calls `Stream::read` (which has a ~100 ms underlying read timeout
///    set by [`crate::rtsp::client::RtspClient::connect_with`]).
/// 3. Releases the mutex.
/// 4. Returns whatever the read returned (`Ok(n)`, `Ok(0)` for EOF, or
///    `Err(WouldBlock|TimedOut)` for the timeout — the pump loops back).
///
/// Holding the mutex for at most ~100 ms per call is fine: RTSP is not
/// pipelined, so the main thread only contends when it's actively
/// sending/receiving a single request, and a 100 ms wait there is
/// invisible compared to typical RTSP round-trip times.
pub(crate) struct SharedStreamReader {
    stream: Arc<Mutex<Stream>>,
}

impl SharedStreamReader {
    pub(crate) fn new(stream: Arc<Mutex<Stream>>) -> Self {
        Self { stream }
    }
}

impl Read for SharedStreamReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut g = self
            .stream
            .lock()
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::Other, "stream mutex poisoned"))?;
        g.read(buf)
    }
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
            Arc::new(AtomicBool::new(false)),
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
            Arc::new(AtomicBool::new(false)),
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
            Arc::new(AtomicBool::new(false)),
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
            Arc::new(AtomicBool::new(false)),
            stats.clone(),
        );
        let msg = cr.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
        assert!(msg.ends_with(b"\r\n\r\nHELLO"));
        let _ = handle.join();
        assert_eq!(stats.rtsp_messages_received.load(Ordering::Relaxed), 1);
    }

    /// B1 review Minor #3: an RTSP response on the interleaved channel that
    /// declares a malformed/oversized Content-Length must CLOSE the pump (not
    /// silently coerce to a 0-length body and desync, nor buffer toward an
    /// uncapped body). Mirrors the server pump's close-on-bad-CL policy at this
    /// call site. Covers both unparseable and over-cap CLs.
    #[test]
    fn rtsp_response_closes_pump_on_malformed_content_length() {
        for header in [
            "RTSP/1.0 200 OK\r\nCSeq: 2\r\nContent-Length: nope\r\n\r\n",
            "RTSP/1.0 200 OK\r\nCSeq: 2\r\nContent-Length: 2000000000\r\n\r\n",
        ] {
            let raw = header.as_bytes().to_vec();
            let (dt, _dr, rt, _rr, ct, cr, cancel, stats) = make_args();
            let handle = spawn_client_pump(
                Cursor::new(raw),
                dt,
                rt,
                ct,
                InterleavedChannels { rtp: 0, rtcp: 1 },
                cancel.clone(),
                Arc::new(AtomicBool::new(false)),
                stats.clone(),
            );
            // The pump must exit on its own (bad CL is fatal) — join returns.
            let _ = handle.join();
            // No RTSP message should have been routed.
            assert!(
                cr.try_recv().is_err(),
                "malformed Content-Length must not produce an RTSP message"
            );
            assert!(
                stats.malformed_frames.load(Ordering::Relaxed) >= 1,
                "malformed Content-Length should be counted as a malformed frame"
            );
            assert_eq!(stats.rtsp_messages_received.load(Ordering::Relaxed), 0);
        }
    }

    /// B2: an RTSP response that never terminates (no `CRLFCRLF`) must NOT
    /// drive the pump to buffer unboundedly — the buffer cap closes the pump
    /// once accumulation exceeds `MAX_RTSP_MESSAGE_BYTES`. This is the
    /// B1-flagged gap: the client pump header buffer was previously uncapped,
    /// so a peer that never sends CRLFCRLF could grow `buf` without bound.
    #[test]
    fn pump_closes_on_unterminated_header_flood() {
        // 128 KiB of header-junk with no CRLFCRLF terminator — well over the
        // 64 KiB cap. Starts with a non-`$` byte so it's parsed as RTSP text.
        let raw = vec![b'A'; 128 * 1024];
        let (dt, _dr, rt, _rr, ct, cr, cancel, stats) = make_args();
        let handle = spawn_client_pump(
            Cursor::new(raw),
            dt,
            rt,
            ct,
            InterleavedChannels { rtp: 0, rtcp: 1 },
            cancel.clone(),
            Arc::new(AtomicBool::new(false)),
            stats.clone(),
        );
        // The pump must exit on its own once the cap is exceeded — join returns.
        let _ = handle.join();
        assert!(
            cr.try_recv().is_err(),
            "unterminated flood must not produce an RTSP message"
        );
        assert!(
            stats.malformed_frames.load(Ordering::Relaxed) >= 1,
            "over-cap buffer should be counted as a malformed frame"
        );
        assert_eq!(stats.rtsp_messages_received.load(Ordering::Relaxed), 0);
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
            Arc::new(AtomicBool::new(false)),
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
            Arc::new(AtomicBool::new(false)),
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
            Arc::new(AtomicBool::new(false)),
            stats.clone(),
        );
        let _ = handle.join();
    }

    /// Regression guard for the 2026-05-28 CI hang in
    /// `client_setup_with_transport_tcp_round_trips_ts` (both Linux gates,
    /// run 26602244548): the pump holds the stream mutex across each
    /// blocking ~100 ms read, so without yielding to `write_gate` an
    /// in-session control write (PLAY/PAUSE) is starved indefinitely on a
    /// contended runner. This asserts the pump STOPS reading while the gate
    /// is set — the precondition that lets a control writer acquire the
    /// lock promptly. Deterministic on any kernel: an un-gated pump keeps
    /// reading (counter climbs); a gated pump freezes.
    #[test]
    fn pump_stops_reading_while_write_gate_set() {
        use std::net::{TcpListener, TcpStream};
        use std::time::{Duration, Instant};

        // Loopback pair whose server side never sends, so every pump read
        // blocks for the full 100 ms read timeout while holding the lock —
        // the idle-control-connection case that monopolizes the mutex.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let done = Arc::new(AtomicBool::new(false));
        let done_srv = done.clone();
        let server = std::thread::spawn(move || {
            let _conn = listener.accept().unwrap();
            // Hold the connection open (never send) until the test is done.
            while !done_srv.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(20));
            }
        });
        let client = TcpStream::connect(addr).unwrap();
        client
            .set_read_timeout(Some(Duration::from_millis(100)))
            .unwrap();
        let stream = Arc::new(Mutex::new(Stream::Plain(client)));

        let (dt, _dr) = mpsc::channel();
        let (rt, _rr) = mpsc::channel();
        let (ct, _cr) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let write_gate = Arc::new(AtomicBool::new(false));
        let stats = Arc::new(PumpStats::default());
        let reader = SharedStreamReader::new(stream.clone());
        let handle = spawn_client_pump(
            reader,
            dt,
            rt,
            ct,
            InterleavedChannels { rtp: 0, rtcp: 1 },
            cancel.clone(),
            write_gate.clone(),
            stats.clone(),
        );

        // Let the pump take a few read cycles.
        std::thread::sleep(Duration::from_millis(50));
        // Ask it to yield, then wait past one in-flight read (100 ms) so any
        // read in progress when we set the gate has drained.
        write_gate.store(true, Ordering::Relaxed);
        std::thread::sleep(Duration::from_millis(200));
        let baseline = stats.reads_attempted.load(Ordering::Relaxed);
        // Over the next 300 ms (~3 read cycles) the pump must not read.
        std::thread::sleep(Duration::from_millis(300));
        let after = stats.reads_attempted.load(Ordering::Relaxed);

        // And a control writer must acquire the lock promptly while gated.
        let t0 = Instant::now();
        {
            let _g = stream.lock().unwrap();
        }
        let lock_wait = t0.elapsed();

        cancel.store(true, Ordering::Relaxed);
        write_gate.store(false, Ordering::Relaxed);
        done.store(true, Ordering::Relaxed);
        let _ = handle.join();
        let _ = server.join();

        assert_eq!(
            after,
            baseline,
            "pump performed {} reads while write_gate was set — it must yield the lock",
            after - baseline
        );
        assert!(
            lock_wait < Duration::from_millis(500),
            "control writer waited {lock_wait:?} for the stream lock while the gate was set"
        );
    }
}
