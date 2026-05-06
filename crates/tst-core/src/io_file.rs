//! File-based ergonomics for the muxer and demuxer.
//!
//! Gated behind the `file` cargo feature (default-on). Embedded users
//! without a filesystem disable via `tst-core = { default-features = false }`.

use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;

use crate::error::DemuxError;
use crate::mpegts::demux::{Demuxer, DemuxEvent, DemuxerOptions};
use crate::mpegts::mux::Muxer;

/// Drain all pending events from a demuxer into `out`.
fn drain_events(demuxer: &mut Demuxer, out: &mut Vec<DemuxEvent>) {
    while let Some(ev) = demuxer.next_event() {
        out.push(ev);
    }
}

/// Read an entire `.ts` file and return all events emitted by the demuxer.
///
/// Convenient for analysis scripts and tests. For large files, prefer
/// [`DemuxFromFile`] (streaming, bounded memory).
///
/// # Errors
///
/// Returns [`std::io::Error`] if the file cannot be read. Demux errors
/// (e.g. unrecoverable sync loss) are mapped to
/// `io::ErrorKind::InvalidData`.
pub fn demux_file(path: impl AsRef<Path>) -> io::Result<Vec<DemuxEvent>> {
    demux_file_with_options(path, DemuxerOptions::default())
}

/// Like [`demux_file`] but with caller-supplied [`DemuxerOptions`].
pub fn demux_file_with_options(
    path: impl AsRef<Path>,
    options: DemuxerOptions,
) -> io::Result<Vec<DemuxEvent>> {
    let bytes = std::fs::read(path)?;
    let mut demuxer = Demuxer::with_options(options);
    let mut out: Vec<DemuxEvent> = Vec::new();
    demuxer
        .feed(&bytes)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, demux_err_msg(e)))?;
    drain_events(&mut demuxer, &mut out);
    demuxer.flush();
    drain_events(&mut demuxer, &mut out);
    Ok(out)
}

/// Streaming iterator over demuxer events from a file. Reads the file
/// in 64 KiB chunks, feeding the demuxer incrementally.
///
/// Bounded memory: at most 64 KiB of raw TS bytes are held at a time,
/// plus whatever the demuxer buffers for in-progress PES reassembly.
///
/// ```no_run
/// use tst_core::io_file::DemuxFromFile;
///
/// let iter = DemuxFromFile::open("input.ts").unwrap();
/// for event in iter {
///     println!("{:?}", event);
/// }
/// ```
pub struct DemuxFromFile {
    file: File,
    buf: Box<[u8; 65536]>,
    demuxer: Demuxer,
    pending: std::collections::VecDeque<DemuxEvent>,
    eof: bool,
}

impl DemuxFromFile {
    /// Open `path` with default [`DemuxerOptions`].
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        Self::open_with_options(path, DemuxerOptions::default())
    }

    /// Open `path` with caller-supplied [`DemuxerOptions`].
    pub fn open_with_options(path: impl AsRef<Path>, options: DemuxerOptions) -> io::Result<Self> {
        Ok(Self {
            file: File::open(path)?,
            buf: Box::new([0u8; 65536]),
            demuxer: Demuxer::with_options(options),
            pending: std::collections::VecDeque::new(),
            eof: false,
        })
    }
}

impl Iterator for DemuxFromFile {
    type Item = DemuxEvent;

    fn next(&mut self) -> Option<DemuxEvent> {
        loop {
            if let Some(ev) = self.pending.pop_front() {
                return Some(ev);
            }
            if self.eof {
                return None;
            }
            let n = match self.file.read(self.buf.as_mut()) {
                Ok(0) => {
                    // EOF: flush the demuxer and collect any trailing events.
                    self.eof = true;
                    self.demuxer.flush();
                    while let Some(ev) = self.demuxer.next_event() {
                        self.pending.push_back(ev);
                    }
                    continue;
                }
                Ok(n) => n,
                Err(_) => {
                    self.eof = true;
                    return None;
                }
            };
            // Feed this chunk; silently stop on unrecoverable demux errors.
            if self.demuxer.feed(&self.buf[..n]).is_err() {
                self.eof = true;
            }
            while let Some(ev) = self.demuxer.next_event() {
                self.pending.push_back(ev);
            }
        }
    }
}

/// Drain a [`Muxer`] to a file. Calls [`Muxer::pull`] repeatedly until
/// the muxer has no more output.
///
/// The muxer must already have content pushed via `push_video` / `push_klv`
/// etc. before calling this. This function does NOT advance the muxer clock
/// or force a PSI flush — the caller is responsible for ensuring the muxer
/// is at the desired boundary before draining.
///
/// # Errors
///
/// Returns [`std::io::Error`] if the file cannot be created or written.
pub fn write_mux_to_file(mux: &mut Muxer, path: impl AsRef<Path>) -> io::Result<()> {
    let mut file = File::create(path)?;
    let mut buf = [0u8; 65536];
    loop {
        let n = mux.pull(&mut buf);
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
    }
    Ok(())
}

fn demux_err_msg(e: DemuxError) -> String {
    format!("demux error: {e}")
}
