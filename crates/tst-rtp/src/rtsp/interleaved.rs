//! TCP-interleaved transport per RFC 7826 §14 (= RFC 2326 §10.12).
//!
//! Wire format on the TCP control connection:
//! - RTSP text: request/response, terminated by CRLFCRLF + optional body.
//! - Binary frame: `0x24` ('$') `<channel:u8> <length:u16 BE> <payload>`.
//!
//! `InterleavedReader` is a sync iterator that yields `Frame::Rtsp(bytes)`
//! or `Frame::Binary { channel, payload }` items as it reads from a
//! `BufRead`. `InterleavedWriter` is mutex-guarded to prevent
//! interleaving of outgoing RTSP text and binary frames.

use std::io::{BufRead, Write};
use std::sync::Mutex;

use bytes::Bytes;

use crate::error::RtspError;
use crate::rtsp::message::{MAX_RTSP_MESSAGE_BYTES, content_length_from_header_text};

/// One demuxed unit from the interleaved TCP stream.
#[derive(Debug, Clone)]
pub enum Frame {
    /// An RTSP request or response (the caller parses it via the
    /// `RtspResponse::parse` helper landed in Task 3). `bytes` includes
    /// the trailing CRLFCRLF + body.
    Rtsp(Bytes),
    /// A binary frame on one of the SETUP-allocated channels.
    Binary { channel: u8, payload: Bytes },
}

/// Maximum binary-frame payload length per RFC 7826 §14 — the 2-byte
/// length field caps a frame at 65535 octets. MPEG-TS-over-RTP at 1316
/// is well under this; we don't fragment.
pub const MAX_BINARY_FRAME_LEN: usize = 65535;

/// Iterator-shaped demuxer. Reads from any `BufRead` impl — in
/// production the `BufReader<TcpStream>`, in tests a `Cursor<&[u8]>`.
///
/// Errors are `RtspError::InterleavedFraming` for malformed binary
/// frames or `RtspError::Io(...)` for TCP failures. EOF is signaled
/// by returning `Ok(None)`.
pub struct InterleavedReader<R: BufRead> {
    inner: R,
}

impl<R: BufRead> InterleavedReader<R> {
    pub fn new(inner: R) -> Self {
        Self { inner }
    }

    /// Read the next frame from the stream.
    ///
    /// Returns:
    /// - `Ok(Some(Frame::Binary { ... }))` for `$<ch><len><payload>` frames
    /// - `Ok(Some(Frame::Rtsp(bytes)))` for full RTSP messages (headers + body)
    /// - `Ok(None)` on clean EOF (no bytes available at frame boundary)
    /// - `Err(RtspError::InterleavedFraming { ... })` for malformed `$`-frames
    /// - `Err(RtspError::Io(...))` for socket I/O errors
    pub fn next_frame(&mut self) -> Result<Option<Frame>, RtspError> {
        // Peek the next byte to determine frame kind.
        let buf = self.inner.fill_buf().map_err(|e| RtspError::Io(e.kind()))?;
        if buf.is_empty() {
            return Ok(None);
        }
        let first = buf[0];
        if first == b'$' {
            self.read_binary_frame()
        } else {
            self.read_rtsp_message()
        }
    }

    fn read_binary_frame(&mut self) -> Result<Option<Frame>, RtspError> {
        let mut header = [0u8; 4];
        self.inner
            .read_exact(&mut header)
            .map_err(|e| RtspError::Io(e.kind()))?;
        if header[0] != b'$' {
            return Err(RtspError::InterleavedFraming {
                detail: "expected $ marker",
            });
        }
        let channel = header[1];
        let length = u16::from_be_bytes([header[2], header[3]]) as usize;
        if length > MAX_BINARY_FRAME_LEN {
            return Err(RtspError::InterleavedFraming {
                detail: "frame length exceeds RFC 7826 §14 max",
            });
        }
        let mut payload = vec![0u8; length];
        self.inner
            .read_exact(&mut payload)
            .map_err(|e| RtspError::Io(e.kind()))?;
        Ok(Some(Frame::Binary {
            channel,
            payload: Bytes::from(payload),
        }))
    }

    fn read_rtsp_message(&mut self) -> Result<Option<Frame>, RtspError> {
        // Read until we see CRLFCRLF terminating headers, then parse
        // Content-Length, then read exactly that many body bytes.
        let mut headers = Vec::new();
        loop {
            let consumed_before = headers.len();
            let buf = self.inner.fill_buf().map_err(|e| RtspError::Io(e.kind()))?;
            if buf.is_empty() {
                return Err(RtspError::BadResponse {
                    detail: "EOF mid-headers",
                });
            }
            // Scan for CRLFCRLF across the existing-headers + just-filled-buf
            // boundary. We can't just scan `buf` alone because the CRLFCRLF
            // could straddle two fill_buf() cycles.
            let combined = [&headers[..], buf].concat();
            if let Some(end) = combined.windows(4).position(|w| w == b"\r\n\r\n") {
                let need = end + 4 - consumed_before;
                headers.extend_from_slice(&buf[..need]);
                self.inner.consume(need);
                break;
            }
            let take = buf.len();
            headers.extend_from_slice(buf);
            self.inner.consume(take);
            // Cap the pre-terminator header accumulation with the shared RTSP
            // message cap. A peer that never sends CRLFCRLF would otherwise
            // drive this buffer unbounded.
            if headers.len() > MAX_RTSP_MESSAGE_BYTES {
                return Err(RtspError::BadResponse {
                    detail: "RTSP headers exceed maximum",
                });
            }
        }
        // Find Content-Length header value
        let header_text = std::str::from_utf8(&headers).map_err(|_| RtspError::BadResponse {
            detail: "non-UTF8 RTSP headers",
        })?;
        // Strict Content-Length: an unparseable, oversized (> cap), or
        // duplicate value is rejected rather than silently treated as a
        // 0-length body (which would desync framing) or an uncapped allocation
        // (OOM DoS via `vec![0u8; content_length]`).
        let content_length = content_length_from_header_text(header_text)
            .map_err(|detail| RtspError::BadResponse { detail })?;
        let mut body = vec![0u8; content_length];
        if content_length > 0 {
            self.inner
                .read_exact(&mut body)
                .map_err(|e| RtspError::Io(e.kind()))?;
        }
        let mut combined = headers;
        combined.extend_from_slice(&body);
        Ok(Some(Frame::Rtsp(Bytes::from(combined))))
    }
}

/// Mutex-guarded writer to the same TCP stream the
/// [`InterleavedReader`] reads from. Outgoing RTSP requests and
/// outgoing binary frames (RTCP RR on the RTCP channel) serialize
/// through this single mutex.
pub struct InterleavedWriter<W: Write> {
    inner: Mutex<W>,
}

impl<W: Write> InterleavedWriter<W> {
    pub fn new(inner: W) -> Self {
        Self {
            inner: Mutex::new(inner),
        }
    }

    /// Write a full RTSP message (already serialized by
    /// `RtspRequest::encode`).
    pub fn write_rtsp(&self, bytes: &[u8]) -> Result<(), RtspError> {
        let mut g = self
            .inner
            .lock()
            .expect("interleaved writer mutex poisoned");
        g.write_all(bytes).map_err(|e| RtspError::Io(e.kind()))?;
        g.flush().map_err(|e| RtspError::Io(e.kind()))?;
        Ok(())
    }

    /// Write a binary `$<channel><length><payload>` frame.
    pub fn write_binary(&self, channel: u8, payload: &[u8]) -> Result<(), RtspError> {
        if payload.len() > MAX_BINARY_FRAME_LEN {
            return Err(RtspError::InterleavedFraming {
                detail: "outgoing payload exceeds 65535",
            });
        }
        let mut g = self
            .inner
            .lock()
            .expect("interleaved writer mutex poisoned");
        let mut header = [b'$', channel, 0, 0];
        header[2..4].copy_from_slice(&(payload.len() as u16).to_be_bytes());
        g.write_all(&header).map_err(|e| RtspError::Io(e.kind()))?;
        g.write_all(payload).map_err(|e| RtspError::Io(e.kind()))?;
        g.flush().map_err(|e| RtspError::Io(e.kind()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn reader_handles_single_binary_frame() {
        // $ <ch=0> <len=4 BE> <0xDE 0xAD 0xBE 0xEF>
        let raw = b"\x24\x00\x00\x04\xDE\xAD\xBE\xEF";
        let mut r = InterleavedReader::new(Cursor::new(&raw[..]));
        let f = r.next_frame().unwrap().unwrap();
        match f {
            Frame::Binary { channel, payload } => {
                assert_eq!(channel, 0);
                assert_eq!(payload.as_ref(), &[0xDE, 0xAD, 0xBE, 0xEF]);
            }
            _ => panic!("expected binary"),
        }
        // EOF
        assert!(r.next_frame().unwrap().is_none());
    }

    #[test]
    fn reader_handles_single_rtsp_message() {
        let raw = b"RTSP/1.0 200 OK\r\nCSeq: 1\r\n\r\n";
        let mut r = InterleavedReader::new(Cursor::new(&raw[..]));
        let f = r.next_frame().unwrap().unwrap();
        match f {
            Frame::Rtsp(b) => assert_eq!(b.as_ref(), raw),
            _ => panic!("expected rtsp"),
        }
    }

    #[test]
    fn reader_handles_interleaved_text_then_binary() {
        let rtsp_part = b"RTSP/1.0 200 OK\r\nCSeq: 1\r\n\r\n";
        let mut bin_part = vec![b'$', 1u8, 0x00, 0x03];
        bin_part.extend_from_slice(b"FOO");
        let mut combined = Vec::new();
        combined.extend_from_slice(rtsp_part);
        combined.extend_from_slice(&bin_part);
        let mut r = InterleavedReader::new(Cursor::new(combined));
        match r.next_frame().unwrap().unwrap() {
            Frame::Rtsp(_) => {}
            _ => panic!("expected rtsp first"),
        }
        match r.next_frame().unwrap().unwrap() {
            Frame::Binary { channel, payload } => {
                assert_eq!(channel, 1);
                assert_eq!(payload.as_ref(), b"FOO");
            }
            _ => panic!("expected binary second"),
        }
    }

    #[test]
    fn reader_handles_rtsp_with_body() {
        let body = b"v=0\r\n";
        let raw = format!("RTSP/1.0 200 OK\r\nContent-Length: {}\r\n\r\n", body.len());
        let mut combined = raw.into_bytes();
        combined.extend_from_slice(body);
        let mut r = InterleavedReader::new(Cursor::new(combined.clone()));
        match r.next_frame().unwrap().unwrap() {
            Frame::Rtsp(b) => assert_eq!(b.as_ref(), combined.as_slice()),
            _ => panic!("expected rtsp"),
        }
    }

    /// B2: an RTSP message whose headers never terminate (no `CRLFCRLF`) must
    /// be rejected once the accumulated header bytes exceed the shared
    /// `MAX_RTSP_MESSAGE_BYTES` cap, rather than buffering unboundedly.
    #[test]
    fn reader_rejects_unterminated_headers() {
        let raw = vec![b'A'; 128 * 1024]; // 128 KiB, no CRLFCRLF.
        let mut r = InterleavedReader::new(Cursor::new(raw));
        let e = r.next_frame().unwrap_err();
        assert!(
            matches!(e, RtspError::BadResponse { .. }),
            "expected BadResponse on over-cap headers, got {e:?}"
        );
    }

    #[test]
    fn writer_writes_binary_frame() {
        let mut sink = Vec::new();
        {
            let w = InterleavedWriter::new(&mut sink);
            w.write_binary(2, &[0x01, 0x02, 0x03]).unwrap();
        }
        assert_eq!(sink, vec![b'$', 2, 0x00, 0x03, 0x01, 0x02, 0x03]);
    }

    #[test]
    fn writer_writes_rtsp() {
        let mut sink = Vec::new();
        {
            let w = InterleavedWriter::new(&mut sink);
            w.write_rtsp(b"OPTIONS rtsp://cam RTSP/1.0\r\n\r\n")
                .unwrap();
        }
        assert_eq!(&sink[..], b"OPTIONS rtsp://cam RTSP/1.0\r\n\r\n");
    }

    #[test]
    fn writer_rejects_overlong_binary() {
        let mut sink = Vec::new();
        let w = InterleavedWriter::new(&mut sink);
        let payload = vec![0u8; 65536];
        let e = w.write_binary(0, &payload).unwrap_err();
        assert!(matches!(e, RtspError::InterleavedFraming { .. }));
    }
}
