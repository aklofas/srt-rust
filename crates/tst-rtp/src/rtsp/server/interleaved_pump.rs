//! Server-side interleaved producer task — reads incoming `$<channel><len><payload>`
//! frames + RTSP requests off the client's TCP read half, demuxes by
//! channel, routes RTP/RTCP frames to the appropriate ingest path, and
//! pushes RTSP request frames to the per-session handler.
//!
//! Mirror of the client-side pump
//! (`crate::rtsp::client::interleaved_pump`). Both close Phase 2's
//! deferred fix 1.
//!
//! Architecture (per-session):
//! - The per-session task owns a `OwnedReadHalf<TcpStream>` (split from
//!   the accepted TCP).
//! - On PLAY when the negotiated transport is TCP-interleaved, the per-
//!   session task SPAWNS this pump task. The pump becomes the SOLE
//!   reader of the TCP read half from that point.
//! - RTSP request bytes go to a `tokio::sync::mpsc::Sender<Bytes>` the
//!   per-session task polls for request bytes (replacing its direct
//!   `tcp.read()` once interleaved-PLAY mode is active).
//! - RTCP `$<channel=N+1>` frames go to an RTCP ingest mpsc; the
//!   per-mount or per-peer RTCP processor receives.
//! - RTP `$<channel=N>` frames are typically SERVER-ONLY-OUTGOING — the
//!   server doesn't expect RTP from clients in our v1 (no recording).
//!   Receive them anyway + counter-tick; future v2 ANNOUNCE/RECORD
//!   sessions would consume.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use bytes::Bytes;
use tokio::io::AsyncReadExt;
use tokio::net::tcp::OwnedReadHalf;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::rtsp::message::content_length_from_header_text;
use crate::rtsp::message::pump_accumulation_exceeded;

/// Stats observable from outside the pump task.
#[derive(Default)]
pub(crate) struct PumpStats {
    pub(crate) rtsp_messages_received: AtomicU64,
    pub(crate) rtp_frames_received: AtomicU64,
    pub(crate) rtcp_frames_received: AtomicU64,
    pub(crate) malformed_frames: AtomicU64,
}

/// Channel allocation passed in at PLAY time — the SETUP handler
/// allocated these in Task 16. Channels are typically (0, 1) for the
/// first session but may differ.
#[derive(Debug, Clone, Copy)]
pub(crate) struct InterleavedChannels {
    pub(crate) rtp: u8,
    pub(crate) rtcp: u8,
}

/// Spawn the server-side interleaved producer pump.
///
/// `read_half` is the OwnedReadHalf of the accepted TcpStream (split via
/// `tcp.into_split()` by the caller). Pump task takes ownership for the
/// session's lifetime.
///
/// `rtsp_tx` — sink for RTSP request bytes (the per-session task's
/// new "read" path once interleaved-PLAY mode is active).
/// `rtcp_tx` — sink for RTCP `$<N+1>` frame payloads (future RTCP
/// ingest; v1 has no consumer but the channel is plumbed).
/// `channels` — SETUP-allocated RTP/RTCP channel pair.
/// `cancel` — flips when stop() or per-session cancel fires.
/// `stats` — observable counters.
#[allow(dead_code)]
pub(crate) fn spawn_server_pump(
    mut read_half: OwnedReadHalf,
    rtsp_tx: mpsc::Sender<Bytes>,
    rtcp_tx: mpsc::Sender<Bytes>,
    channels: InterleavedChannels,
    cancel: CancellationToken,
    stats: Arc<PumpStats>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut buf: Vec<u8> = Vec::with_capacity(16384);
        let mut chunk = [0u8; 4096];
        loop {
            tokio::select! {
                read_res = read_half.read(&mut chunk) => match read_res {
                    Ok(0) => {
                        // Clean EOF — client disconnected.
                        return;
                    }
                    Ok(n) => buf.extend_from_slice(&chunk[..n]),
                    Err(e) => {
                        tracing::warn!(
                            target: "tst_rtp::server::pump",
                            error = %e,
                            "TCP read failed; pump exiting"
                        );
                        return;
                    }
                },
                _ = cancel.cancelled() => return,
            }

            // Buffer cap: coherent with the session loop and the client pump
            // (shared `pump_accumulation_exceeded` + the same
            // MAX_RTSP_MESSAGE_BYTES / MAX_RTSP_BODY_BYTES constants). This
            // channel interleaves RTSP text frames with binary `$`-frames. A
            // full u16 binary frame (65535-byte payload + 4-byte framing =
            // 65539 B) and an RTSP message with a body up to 1 MiB are BOTH
            // legitimate and no longer falsely rejected; only an unterminated
            // header run > 64 KiB or a bad/over-cap Content-Length closes the
            // pump (no 413 to send mid-stream on an interleaved channel).
            if pump_accumulation_exceeded(&buf) {
                tracing::warn!(
                    target: "tst_rtp::server::pump",
                    buf_len = buf.len(),
                    "pump buffer exceeded RTSP header/body caps; closing"
                );
                stats.malformed_frames.fetch_add(1, Ordering::Relaxed);
                return;
            }

            // Parse as many complete frames as possible from buf.
            loop {
                if buf.is_empty() {
                    break;
                }
                if buf[0] == b'$' {
                    // Binary frame: need 4 header bytes + length payload.
                    if buf.len() < 4 {
                        break; // need more
                    }
                    let channel = buf[1];
                    let length = u16::from_be_bytes([buf[2], buf[3]]) as usize;
                    if buf.len() < 4 + length {
                        break; // need more
                    }
                    let payload = Bytes::copy_from_slice(&buf[4..4 + length]);
                    buf.drain(..4 + length);
                    if channel == channels.rtp {
                        stats.rtp_frames_received.fetch_add(1, Ordering::Relaxed);
                        // v1 server doesn't consume client-sent RTP.
                        // (No ANNOUNCE/RECORD support.) Future v2 routes here.
                    } else if channel == channels.rtcp {
                        stats.rtcp_frames_received.fetch_add(1, Ordering::Relaxed);
                        if rtcp_tx.try_send(payload).is_err() {
                            // RTCP receiver dropped or saturated; drop frame.
                            // v1 doesn't track RTCP-drop stats here.
                        }
                    } else {
                        // Unknown channel — drop + tracing::warn (per spec,
                        // server MUST tolerate unknown channels).
                        tracing::warn!(
                            target: "tst_rtp::server::pump",
                            channel = channel,
                            "binary frame on unknown channel; dropping"
                        );
                        stats.malformed_frames.fetch_add(1, Ordering::Relaxed);
                    }
                } else {
                    // RTSP message: find CRLFCRLF.
                    let end = match buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        Some(e) => e,
                        None => break, // need more
                    };
                    // Find Content-Length to know body size.
                    let header_text = match std::str::from_utf8(&buf[..end]) {
                        Ok(s) => s,
                        Err(_) => {
                            stats.malformed_frames.fetch_add(1, Ordering::Relaxed);
                            buf.drain(..end + 4);
                            continue;
                        }
                    };
                    // Strict Content-Length: unparseable, oversized (> cap), or
                    // duplicate are all hostile. We cannot simply drain the
                    // headers and continue on a bad declared length: the declared
                    // body bytes still sit at the front of `buf`, don't begin with
                    // `$`, re-enter this RTSP-text branch, never find a CRLFCRLF,
                    // and the outer loop keeps reading forever (unbounded growth /
                    // desync). The safe answer on an interleaved control channel
                    // is to close the pump.
                    let content_length = match content_length_from_header_text(header_text) {
                        Ok(n) => n,
                        Err(detail) => {
                            tracing::warn!(
                                target: "tst_rtp::server::pump",
                                detail,
                                "malformed RTSP Content-Length on interleaved channel; closing pump"
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
                        break; // need more
                    }
                    let msg = Bytes::copy_from_slice(&buf[..msg_end]);
                    buf.drain(..msg_end);
                    stats.rtsp_messages_received.fetch_add(1, Ordering::Relaxed);
                    if rtsp_tx.try_send(msg).is_err() {
                        // Per-session task isn't reading — apply TCP
                        // backpressure by not reading more (we naturally
                        // do since our select! waits on read). For now,
                        // just log + continue.
                        tracing::warn!(
                            target: "tst_rtp::server::pump",
                            "rtsp_tx mpsc full or closed"
                        );
                    }
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;
    use tokio::net::{TcpListener, TcpStream};

    /// Feed one binary frame on the RTCP channel; verify it reaches rtcp_tx.
    #[tokio::test]
    async fn pump_delivers_binary_to_rtcp_channel() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let client_join = tokio::spawn(async move {
            let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            // $<channel=1><length=4 BE><DEADBEEF>
            s.write_all(b"\x24\x01\x00\x04\xDE\xAD\xBE\xEF")
                .await
                .unwrap();
            // Hold open so the pump doesn't see EOF.
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        });
        let (server_tcp, _) = listener.accept().await.unwrap();
        let (read_half, _write_half) = server_tcp.into_split();
        let (rtsp_tx, mut _rtsp_rx) = mpsc::channel(8);
        let (rtcp_tx, mut rtcp_rx) = mpsc::channel(8);
        let cancel = CancellationToken::new();
        let stats = Arc::new(PumpStats::default());
        let pump = spawn_server_pump(
            read_half,
            rtsp_tx,
            rtcp_tx,
            InterleavedChannels { rtp: 0, rtcp: 1 },
            cancel.clone(),
            stats.clone(),
        );
        let frame = tokio::time::timeout(std::time::Duration::from_secs(2), rtcp_rx.recv())
            .await
            .expect("timeout")
            .unwrap();
        assert_eq!(frame.as_ref(), &[0xDE, 0xAD, 0xBE, 0xEF]);
        cancel.cancel();
        let _ = pump.await;
        let _ = client_join.await;
        assert_eq!(stats.rtcp_frames_received.load(Ordering::Relaxed), 1);
    }

    /// B7 (T3-PUMP-FRAME): a FULL u16 binary interleaved frame — 65535-byte
    /// payload + 4-byte `$`/channel/len framing = 65539 B — must be ACCEPTED,
    /// not falsely rejected. Before B7 the pump capped `buf` at 64 KiB, so any
    /// frame larger than ~65532 B tripped the cap and closed the pump. A u16
    /// length field permits exactly 65535 payload bytes, so this is the largest
    /// legal frame and must round-trip.
    #[tokio::test]
    async fn pump_accepts_full_u16_binary_frame() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let client_join = tokio::spawn(async move {
            let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            // $<channel=1 (rtcp)><length=65535 BE><65535 payload bytes>
            let mut frame = Vec::with_capacity(65539);
            frame.extend_from_slice(&[0x24, 0x01, 0xFF, 0xFF]);
            frame.extend(std::iter::repeat_n(0xAB, 65535));
            s.write_all(&frame).await.unwrap();
            // Hold open so the pump doesn't see EOF before delivering.
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        });
        let (server_tcp, _) = listener.accept().await.unwrap();
        let (read_half, _write_half) = server_tcp.into_split();
        let (rtsp_tx, mut _rtsp_rx) = mpsc::channel(8);
        let (rtcp_tx, mut rtcp_rx) = mpsc::channel(8);
        let cancel = CancellationToken::new();
        let stats = Arc::new(PumpStats::default());
        let pump = spawn_server_pump(
            read_half,
            rtsp_tx,
            rtcp_tx,
            InterleavedChannels { rtp: 0, rtcp: 1 },
            cancel.clone(),
            stats.clone(),
        );
        let frame = tokio::time::timeout(std::time::Duration::from_secs(2), rtcp_rx.recv())
            .await
            .expect("full u16 frame must be delivered, not rejected by the cap")
            .unwrap();
        assert_eq!(frame.len(), 65535, "full payload must round-trip intact");
        assert!(frame.iter().all(|&b| b == 0xAB));
        cancel.cancel();
        let _ = pump.await;
        let _ = client_join.await;
        assert_eq!(stats.rtcp_frames_received.load(Ordering::Relaxed), 1);
        assert_eq!(stats.malformed_frames.load(Ordering::Relaxed), 0);
    }

    /// Feed a complete RTSP request via the pump; verify it reaches rtsp_tx.
    #[tokio::test]
    async fn pump_delivers_rtsp_message() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let client_join = tokio::spawn(async move {
            let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            s.write_all(b"OPTIONS rtsp://x RTSP/1.0\r\nCSeq: 1\r\n\r\n")
                .await
                .unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        });
        let (server_tcp, _) = listener.accept().await.unwrap();
        let (read_half, _write_half) = server_tcp.into_split();
        let (rtsp_tx, mut rtsp_rx) = mpsc::channel(8);
        let (rtcp_tx, mut _rtcp_rx) = mpsc::channel(8);
        let cancel = CancellationToken::new();
        let stats = Arc::new(PumpStats::default());
        let pump = spawn_server_pump(
            read_half,
            rtsp_tx,
            rtcp_tx,
            InterleavedChannels { rtp: 0, rtcp: 1 },
            cancel.clone(),
            stats.clone(),
        );
        let msg = tokio::time::timeout(std::time::Duration::from_secs(2), rtsp_rx.recv())
            .await
            .expect("timeout")
            .unwrap();
        let text = std::str::from_utf8(&msg).unwrap();
        assert!(text.starts_with("OPTIONS"));
        cancel.cancel();
        let _ = pump.await;
        let _ = client_join.await;
        assert_eq!(stats.rtsp_messages_received.load(Ordering::Relaxed), 1);
    }

    /// Mix RTSP message + binary frame in one TCP write; pump demuxes both.
    #[tokio::test]
    async fn pump_demuxes_rtsp_then_binary() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let client_join = tokio::spawn(async move {
            let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            let mut buf = Vec::new();
            buf.extend_from_slice(b"OPTIONS rtsp://x RTSP/1.0\r\nCSeq: 1\r\n\r\n");
            buf.extend_from_slice(b"\x24\x01\x00\x03FOO");
            s.write_all(&buf).await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        });
        let (server_tcp, _) = listener.accept().await.unwrap();
        let (read_half, _write_half) = server_tcp.into_split();
        let (rtsp_tx, mut rtsp_rx) = mpsc::channel(8);
        let (rtcp_tx, mut rtcp_rx) = mpsc::channel(8);
        let cancel = CancellationToken::new();
        let stats = Arc::new(PumpStats::default());
        let pump = spawn_server_pump(
            read_half,
            rtsp_tx,
            rtcp_tx,
            InterleavedChannels { rtp: 0, rtcp: 1 },
            cancel.clone(),
            stats.clone(),
        );
        let rtsp = tokio::time::timeout(std::time::Duration::from_secs(2), rtsp_rx.recv())
            .await
            .expect("timeout")
            .unwrap();
        assert!(rtsp.starts_with(b"OPTIONS"));
        let bin = tokio::time::timeout(std::time::Duration::from_secs(2), rtcp_rx.recv())
            .await
            .expect("timeout")
            .unwrap();
        assert_eq!(bin.as_ref(), b"FOO");
        cancel.cancel();
        let _ = pump.await;
        let _ = client_join.await;
    }

    /// Regression: an RTSP message on the interleaved channel that declares
    /// an oversized Content-Length must CLOSE the pump (not desync into an
    /// unbounded-growth loop). Before the fix, the oversized-CL branch drained
    /// only the headers, leaving the declared body bytes at the front of `buf`;
    /// they re-entered the RTSP-text branch, never found CRLFCRLF, and the
    /// outer loop read forever → `buf` grew unbounded. The pump must now
    /// terminate promptly (watchdog-bounded, deterministic).
    #[tokio::test]
    async fn pump_closes_on_oversized_content_length() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let client_join = tokio::spawn(async move {
            let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            // Declare a 2 GB body, then send a little junk after the header
            // block. The pump must close on seeing the oversized CL — it must
            // NOT wait for (or buffer toward) the impossible 2 GB body.
            let mut buf = Vec::new();
            buf.extend_from_slice(
                b"OPTIONS rtsp://x RTSP/1.0\r\nCSeq: 1\r\nContent-Length: 2000000000\r\n\r\n",
            );
            buf.extend_from_slice(&[0x42u8; 4096]);
            // Ignore write errors — the pump may close the read half first.
            let _ = s.write_all(&buf).await;
            // Hold the socket briefly so the FIN doesn't masquerade as the
            // close cause; the cap/close logic is what we're testing.
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        });
        let (server_tcp, _) = listener.accept().await.unwrap();
        let (read_half, _write_half) = server_tcp.into_split();
        let (rtsp_tx, mut _rtsp_rx) = mpsc::channel(8);
        let (rtcp_tx, mut _rtcp_rx) = mpsc::channel(8);
        let cancel = CancellationToken::new();
        let stats = Arc::new(PumpStats::default());
        let pump = spawn_server_pump(
            read_half,
            rtsp_tx,
            rtcp_tx,
            InterleavedChannels { rtp: 0, rtcp: 1 },
            cancel.clone(),
            stats.clone(),
        );
        // The pump must terminate on its own (without us cancelling) because
        // the oversized CL is fatal. Watchdog: 3 s ceiling — far below any
        // unbounded-growth runaway.
        tokio::time::timeout(std::time::Duration::from_secs(3), pump)
            .await
            .expect("pump did not close on oversized Content-Length (OOM-growth regression)")
            .ok();
        assert!(
            stats.malformed_frames.load(Ordering::Relaxed) >= 1,
            "oversized Content-Length should be counted as a malformed frame"
        );
        let _ = client_join.await;
    }

    /// Regression: a flood of non-`$`, non-CRLFCRLF junk on the interleaved
    /// channel must trip the pump-level buffer cap and CLOSE the pump rather
    /// than reading forever (the pump's `buf` previously had no size cap at
    /// all). Watchdog-bounded + deterministic.
    #[tokio::test]
    async fn pump_closes_on_unbounded_junk() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let client_join = tokio::spawn(async move {
            let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            // 256 KiB of junk that never contains CRLFCRLF and never starts a
            // binary frame — exceeds the 64 KiB unterminated-header cap
            // (MAX_RTSP_MESSAGE_BYTES). Use 'A' so it stays in the RTSP-text
            // branch (not '$').
            let junk = vec![b'A'; 256 * 1024];
            let _ = s.write_all(&junk).await;
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        });
        let (server_tcp, _) = listener.accept().await.unwrap();
        let (read_half, _write_half) = server_tcp.into_split();
        let (rtsp_tx, mut _rtsp_rx) = mpsc::channel(8);
        let (rtcp_tx, mut _rtcp_rx) = mpsc::channel(8);
        let cancel = CancellationToken::new();
        let stats = Arc::new(PumpStats::default());
        let pump = spawn_server_pump(
            read_half,
            rtsp_tx,
            rtcp_tx,
            InterleavedChannels { rtp: 0, rtcp: 1 },
            cancel.clone(),
            stats.clone(),
        );
        tokio::time::timeout(std::time::Duration::from_secs(3), pump)
            .await
            .expect("pump did not close on unbounded junk (buffer-cap regression)")
            .ok();
        assert!(
            stats.malformed_frames.load(Ordering::Relaxed) >= 1,
            "buffer-cap trip should be counted as a malformed frame"
        );
        let _ = client_join.await;
    }
}
