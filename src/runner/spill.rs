//! Per-stream spill-to-disk state machine. New code — rtk read child output
//! line by line into an unbounded `String` behind a 10 MiB truncate-with-
//! warning cap (`RAW_CAP`), which is exactly the loss this design removes.
//!
//! Two states, one per stream:
//!
//! * **Buffering** — raw bytes accumulate in memory while the running total
//!   stays under the token gate. If the child ends here, those bytes are the
//!   passthrough output, byte for byte.
//! * **Spilling** — on crossing the gate the fold file is opened once, the
//!   buffered prefix is written, and every later chunk goes straight to disk.
//!   Memory stops growing: from then on only a byte count, a newline count
//!   and a bounded tail ring are kept.
//!
//! The head needs no tracking. `SLICE_MAX_BYTES` (8 KB) is far below the
//! default gate (~100 KB), so the head is still sitting in the pre-spill
//! buffer at the moment of crossing — it is sliced there, once.

use std::fs::File;
use std::io::{self, BufWriter, Read, Write};
use std::path::PathBuf;

use crate::fold::summary::{Fold, SLICE_MAX_BYTES, TAIL_LINES, total_lines_from};
use crate::fold::tokens::estimate_tokens_len;
use crate::fold::write::open_fold_file;

/// Read size per `read` call on a child pipe.
const READ_CHUNK: usize = 64 * 1024;

/// Upper bound on the tail ring before it is trimmed back. Trimming at twice
/// the slice cap keeps the work amortized O(1) per byte: each byte is copied
/// at most once more, no matter how long the stream runs.
const TAIL_RING_MAX: usize = 2 * SLICE_MAX_BYTES;

/// What one stream produced.
pub(crate) enum StreamCapture {
    /// Stayed under the gate: these are the child's bytes, to be written to
    /// the real fd untouched.
    Passthrough(Vec<u8>),
    /// Crossed the gate: full output is on disk, this is the summary of it.
    Folded(Fold),
}

impl StreamCapture {
    /// Raw byte count the command actually produced on this stream. For a
    /// passthrough this is `kept_bytes` too — the tracking row's mandatory
    /// evidence that "no fold happened" is not the same as "zero bytes".
    pub(crate) fn raw_bytes(&self) -> usize {
        match self {
            StreamCapture::Passthrough(bytes) => bytes.len(),
            StreamCapture::Folded(fold) => fold.raw_bytes,
        }
    }

    /// Bytes that actually reached the caller: all of them for a
    /// passthrough, only the compact summary for a fold.
    pub(crate) fn kept_bytes(&self) -> usize {
        match self {
            StreamCapture::Passthrough(bytes) => bytes.len(),
            StreamCapture::Folded(fold) => fold.kept_bytes,
        }
    }

    pub(crate) fn is_folded(&self) -> bool {
        matches!(self, StreamCapture::Folded(_))
    }

    /// Path to the full output on disk, if this stream folded.
    pub(crate) fn fold_path(&self) -> Option<&std::path::Path> {
        match self {
            StreamCapture::Passthrough(_) => None,
            StreamCapture::Folded(fold) => Some(&fold.path),
        }
    }
}

enum State {
    Buffering(Vec<u8>),
    Spilling(BufWriter<File>),
}

/// Size gate and destination for one stream. stdout and stderr each own one:
/// separate threshold evaluation, separate fold file, separate destination
/// fd. They are never merged or interleaved.
pub(crate) struct StreamSink {
    /// Slug for the fold filename, already suffixed per stream by the caller
    /// so both streams crossing in the same second cannot collide.
    slug: String,
    /// `None` when no fold directory could be resolved. Only an error if the
    /// stream actually crosses the gate — a small run must still work on a
    /// platform with no data directory.
    dir: Option<PathBuf>,
    threshold_tokens: usize,
    enabled: bool,
    state: State,
    path: Option<PathBuf>,
    raw_bytes: usize,
    newlines: usize,
    last_byte: Option<u8>,
    /// First `SLICE_MAX_BYTES` bytes, captured at the moment of spilling.
    head: Vec<u8>,
    /// Trailing window, bounded by `TAIL_RING_MAX`.
    tail: Vec<u8>,
}

impl StreamSink {
    pub(crate) fn new(
        slug: String,
        dir: Option<PathBuf>,
        threshold_tokens: usize,
        enabled: bool,
    ) -> Self {
        Self {
            slug,
            dir,
            threshold_tokens,
            enabled,
            state: State::Buffering(Vec::new()),
            path: None,
            raw_bytes: 0,
            newlines: 0,
            last_byte: None,
            head: Vec::new(),
            tail: Vec::new(),
        }
    }

    /// Feed one chunk of raw child bytes.
    pub(crate) fn feed(&mut self, chunk: &[u8]) -> io::Result<()> {
        if chunk.is_empty() {
            return Ok(());
        }
        self.raw_bytes += chunk.len();
        // Count newlines instead of materializing lines: the line total must
        // stay exact for output that never fits in memory.
        self.newlines += chunk.iter().filter(|b| **b == b'\n').count();
        self.last_byte = chunk.last().copied();

        let crossed = match &mut self.state {
            State::Buffering(buf) => {
                buf.extend_from_slice(chunk);
                self.enabled && estimate_tokens_len(self.raw_bytes) >= self.threshold_tokens
            }
            State::Spilling(writer) => {
                writer.write_all(chunk)?;
                self.tail.extend_from_slice(chunk);
                trim_tail(&mut self.tail);
                false
            }
        };
        if crossed {
            self.spill()?;
        }
        Ok(())
    }

    /// Cross into the spilling state: open the fold file, snapshot head and
    /// tail out of the buffer, flush the buffered prefix to disk, drop it.
    fn spill(&mut self) -> io::Result<()> {
        let State::Buffering(buf) = &self.state else {
            return Ok(());
        };
        let dir = self.dir.as_ref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "no fold directory available (no data_local_dir on this platform)",
            )
        })?;

        let (file, path) = open_fold_file(&self.slug, dir)?;
        let mut writer = BufWriter::new(file);
        writer.write_all(buf)?;

        // Head and tail come out of the buffer here, the one moment both are
        // still in memory. After this the buffer is dropped and nothing
        // re-reads the file.
        self.head
            .extend_from_slice(&buf[..SLICE_MAX_BYTES.min(buf.len())]);
        self.tail
            .extend_from_slice(&buf[buf.len().saturating_sub(TAIL_RING_MAX)..]);
        trim_tail(&mut self.tail);

        self.path = Some(path);
        self.state = State::Spilling(writer);
        Ok(())
    }

    /// Drain `reader` to EOF into this sink, then finish.
    pub(crate) fn pump(mut self, mut reader: impl Read) -> io::Result<StreamCapture> {
        let mut chunk = vec![0u8; READ_CHUNK];
        loop {
            match reader.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => self.feed(&chunk[..n])?,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
        self.finish()
    }

    fn finish(self) -> io::Result<StreamCapture> {
        match self.state {
            State::Buffering(buf) => Ok(StreamCapture::Passthrough(buf)),
            State::Spilling(mut writer) => {
                writer.flush()?;
                let path = self
                    .path
                    .ok_or_else(|| io::Error::other("spilled stream has no fold file path"))?;
                Ok(StreamCapture::Folded(Fold::from_regions(
                    &self.head,
                    &self.tail,
                    total_lines_from(self.newlines, self.last_byte),
                    self.raw_bytes,
                    path,
                )))
            }
        }
    }
}

/// Bound the tail ring: past `TAIL_RING_MAX` bytes, keep the last
/// `SLICE_MAX_BYTES` and then only the last `TAIL_LINES` lines of those.
/// Never drops anything the fold could display — the summary caps the tail
/// at the same byte and line limits.
///
/// Computes the single cut point up front and drains once — draining the
/// byte cap and then the line cap separately would memmove the retained
/// bytes twice per trim instead of once.
fn trim_tail(tail: &mut Vec<u8>) {
    if tail.len() <= TAIL_RING_MAX {
        return;
    }
    let byte_cut = tail.len() - SLICE_MAX_BYTES;

    // Keep one newline more than TAIL_LINES so the boundary line stays whole.
    let mut seen = 0;
    let mut cut = byte_cut;
    for i in (byte_cut..tail.len()).rev() {
        if tail[i] == b'\n' {
            seen += 1;
            if seen > TAIL_LINES {
                cut = i + 1;
                break;
            }
        }
    }
    tail.drain(..cut);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sink(dir: &std::path::Path, threshold: usize) -> StreamSink {
        StreamSink::new("test".to_string(), Some(dir.to_path_buf()), threshold, true)
    }

    #[test]
    fn stays_buffered_below_the_gate() {
        let tmp = tempfile::tempdir().unwrap();
        let raw = b"small output\n";
        let capture = sink(tmp.path(), 1_000).pump(&raw[..]).unwrap();
        match capture {
            StreamCapture::Passthrough(bytes) => assert_eq!(bytes, raw),
            StreamCapture::Folded(_) => panic!("must not fold below the gate"),
        }
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 0);
    }

    #[test]
    fn disabled_never_spills_however_large() {
        let tmp = tempfile::tempdir().unwrap();
        let raw = vec![b'x'; 200_000];
        let s = StreamSink::new("test".into(), Some(tmp.path().into()), 10, false);
        let capture = s.pump(&raw[..]).unwrap();
        assert!(matches!(capture, StreamCapture::Passthrough(_)));
    }

    #[test]
    fn spilled_file_is_byte_identical_to_the_input() {
        let tmp = tempfile::tempdir().unwrap();
        let raw: Vec<u8> = (0..40_000)
            .flat_map(|i| format!("line {i}\n").into_bytes())
            .collect();
        let capture = sink(tmp.path(), 100).pump(&raw[..]).unwrap();
        let StreamCapture::Folded(fold) = capture else {
            panic!("expected a fold");
        };
        assert_eq!(std::fs::read(&fold.path).unwrap(), raw);
        assert_eq!(fold.raw_bytes, raw.len());
        assert_eq!(fold.total_lines, 40_000);
    }

    #[test]
    fn head_and_tail_survive_a_stream_far_larger_than_memory_window() {
        let tmp = tempfile::tempdir().unwrap();
        let raw: Vec<u8> = (0..40_000)
            .flat_map(|i| format!("line {i}\n").into_bytes())
            .collect();
        let capture = sink(tmp.path(), 100).pump(&raw[..]).unwrap();
        let StreamCapture::Folded(fold) = capture else {
            panic!("expected a fold");
        };
        assert!(fold.head.starts_with("line 0\nline 1\n"));
        assert_eq!(fold.head.lines().count(), 50);
        assert!(fold.tail.ends_with("line 39999"));
        assert_eq!(fold.tail.lines().count(), 50);
    }

    #[test]
    fn folding_matches_the_in_memory_path_byte_for_byte() {
        // The single-constructor guarantee, end to end: streaming a buffer
        // through the sink must produce the same head/tail/counts as folding
        // the same buffer in memory.
        let tmp = tempfile::tempdir().unwrap();
        let raw: String = (0..5_000).map(|i| format!("line {i}\n")).collect();
        let capture = sink(tmp.path(), 100).pump(raw.as_bytes()).unwrap();
        let StreamCapture::Folded(streamed) = capture else {
            panic!("expected a fold");
        };

        let cfg = crate::fold::FoldConfig {
            threshold_tokens: 100,
            directory: Some(tmp.path().to_path_buf()),
            ..crate::fold::FoldConfig::default()
        };
        let crate::fold::FoldOutcome::Folded(in_memory) =
            crate::fold::fold_output(&raw, "test", &cfg).unwrap()
        else {
            panic!("expected a fold");
        };

        assert_eq!(streamed.head, in_memory.head);
        assert_eq!(streamed.tail, in_memory.tail);
        assert_eq!(streamed.total_lines, in_memory.total_lines);
        assert_eq!(streamed.raw_bytes, in_memory.raw_bytes);
        assert_eq!(streamed.kept_bytes, in_memory.kept_bytes);
    }

    #[test]
    fn single_huge_line_keeps_head_only() {
        let tmp = tempfile::tempdir().unwrap();
        let raw = vec![b'x'; 200_000];
        let capture = sink(tmp.path(), 100).pump(&raw[..]).unwrap();
        let StreamCapture::Folded(fold) = capture else {
            panic!("expected a fold");
        };
        assert_eq!(fold.total_lines, 1);
        assert!(fold.tail.is_empty(), "one line leaves nothing for the tail");
        assert!(fold.head.len() <= SLICE_MAX_BYTES);
    }

    #[test]
    fn invalid_utf8_spills_losslessly_and_previews_lossily() {
        let tmp = tempfile::tempdir().unwrap();
        let mut raw = Vec::new();
        for i in 0..20_000 {
            raw.extend_from_slice(&[0xff, 0xfe, b'a' + (i % 26) as u8, b'\n']);
        }
        let capture = sink(tmp.path(), 100).pump(&raw[..]).unwrap();
        let StreamCapture::Folded(fold) = capture else {
            panic!("expected a fold");
        };
        assert_eq!(
            std::fs::read(&fold.path).unwrap(),
            raw,
            "persisted bytes must be exactly what the stream carried"
        );
        assert!(fold.head.len() <= SLICE_MAX_BYTES);
        assert!(fold.tail.len() <= SLICE_MAX_BYTES);
    }

    #[test]
    fn tail_ring_stays_bounded_across_many_chunks() {
        let tmp = tempfile::tempdir().unwrap();
        let mut s = sink(tmp.path(), 10);
        for i in 0..2_000 {
            s.feed(format!("chunk {i} padding padding padding\n").as_bytes())
                .unwrap();
        }
        assert!(
            s.tail.len() <= TAIL_RING_MAX,
            "tail ring grew to {}",
            s.tail.len()
        );
        assert!(matches!(s.state, State::Spilling(_)), "must have spilled");
    }

    #[test]
    fn accessors_report_passthrough_bytes_before_emit_consumes_the_capture() {
        let tmp = tempfile::tempdir().unwrap();
        let raw = b"small output\n";
        let capture = sink(tmp.path(), 1_000).pump(&raw[..]).unwrap();
        assert!(!capture.is_folded());
        assert_eq!(capture.raw_bytes(), raw.len());
        assert_eq!(capture.kept_bytes(), raw.len());
        assert!(capture.fold_path().is_none());
    }

    #[test]
    fn accessors_report_folded_raw_and_kept_bytes() {
        let tmp = tempfile::tempdir().unwrap();
        let raw: Vec<u8> = (0..40_000)
            .flat_map(|i| format!("line {i}\n").into_bytes())
            .collect();
        let capture = sink(tmp.path(), 100).pump(&raw[..]).unwrap();
        assert!(capture.is_folded());
        assert_eq!(capture.raw_bytes(), raw.len());
        assert!(capture.kept_bytes() < capture.raw_bytes());
        assert!(capture.fold_path().is_some());
    }

    #[test]
    fn empty_stream_is_empty_passthrough() {
        let tmp = tempfile::tempdir().unwrap();
        let capture = sink(tmp.path(), 10).pump(&b""[..]).unwrap();
        match capture {
            StreamCapture::Passthrough(bytes) => assert!(bytes.is_empty()),
            StreamCapture::Folded(_) => panic!("empty stream must not fold"),
        }
    }
}
