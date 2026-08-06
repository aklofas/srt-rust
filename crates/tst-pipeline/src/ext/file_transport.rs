//! [`FileTransport`] — write-to-file [`Transport`]: the natural capture /
//! debug sink for [`MuxSender`](crate::MuxSender) (both binding consumers
//! and integration ports have independently rebuilt this — ship it once).

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use tst_core::transport::{SocketStats, Transport, TransportError};

/// Appends every chunk verbatim to a file. `close` flushes; post-close
/// sends return [`TransportError::Closed`] per the trait contract.
pub struct FileTransport {
    writer: Option<BufWriter<File>>,
    bytes_sent: u64,
}

impl FileTransport {
    /// Create (truncating) `path` and return a transport writing to it.
    pub fn create(path: impl AsRef<Path>) -> std::io::Result<Self> {
        Ok(Self {
            writer: Some(BufWriter::new(File::create(path)?)),
            bytes_sent: 0,
        })
    }

    /// Total bytes accepted by `send_bytes` (pre-flush count).
    #[must_use]
    pub fn bytes_sent(&self) -> u64 {
        self.bytes_sent
    }
}

impl Transport for FileTransport {
    fn send_bytes(&mut self, msg: &[u8]) -> Result<(), TransportError> {
        let Some(w) = self.writer.as_mut() else {
            return Err(TransportError::Closed);
        };
        w.write_all(msg).map_err(|e| TransportError::Broken {
            msg: format!("file write failed: {e}"),
            errno_code: e.raw_os_error(),
        })?;
        self.bytes_sent = self.bytes_sent.saturating_add(msg.len() as u64);
        Ok(())
    }

    // MuxSender sizes its drain scratch to max_payload; 7×188 keeps the
    // chunking identical to the wire transports' ecosystem default.
    fn max_payload(&self) -> usize {
        7 * 188
    }

    fn is_alive(&self) -> bool {
        self.writer.is_some()
    }

    fn close(&mut self) {
        if let Some(mut w) = self.writer.take() {
            let _ = w.flush();
        }
    }

    fn socket_stats(&self) -> Option<SocketStats> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tst_core::mpegts::common::Pts90khz;
    use tst_core::mpegts::mux::{MuxerConfig, MuxerProgramConfigBuilder, VideoCodec};

    fn video_only_config() -> MuxerConfig {
        let mut prog = MuxerProgramConfigBuilder::new(1, 0x1000);
        prog.add_video(0x1011, VideoCodec::H264);
        prog.pcr_pid(0x1011);
        let mut b = MuxerConfig::builder();
        b.add_program(prog.build());
        b.build().unwrap()
    }

    #[test]
    fn file_transport_captures_muxed_ts_and_rejects_post_close() {
        let path =
            std::env::temp_dir().join(format!("tst-file-transport-{}.ts", std::process::id()));
        let t = FileTransport::create(&path).unwrap();
        let sender = crate::MuxSender::new(t, video_only_config()).unwrap();
        let nal = [0x00, 0x00, 0x00, 0x01, 0x67, 0xBB];
        sender.send_video(&nal, Pts90khz::new(0), true).unwrap();
        let mut t = sender.into_inner();
        t.close();
        let bytes = std::fs::read(&path).unwrap();
        assert!(!bytes.is_empty() && bytes.len() % 188 == 0 && bytes[0] == 0x47);
        // Post-close contract from the Transport trait rustdoc:
        assert!(matches!(
            t.send_bytes(&[0u8; 188]),
            Err(TransportError::Closed)
        ));
        let _ = std::fs::remove_file(&path);
    }
}
