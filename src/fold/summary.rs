//! The fold itself: new code — rtk never truncates output back to its
//! caller, it only writes a recovery hint alongside the untouched output.
//! Here, output past the token gate is replaced by a compact head/tail
//! summary; the full output is always on disk, untruncated, via `write`.

use std::io;
use std::path::PathBuf;

use super::config::FoldConfig;
use super::paths::{format_hint, format_tail_hint, resolve_fold_dir};
use super::tokens::estimate_tokens;
use super::write::write_fold_file;

/// Lines kept from the start of output when folding.
pub(crate) const HEAD_LINES: usize = 50;
/// Lines kept from the end of output when folding.
pub(crate) const TAIL_LINES: usize = 50;
/// Byte cap per kept slice (head/tail), guarding against a single huge
/// line (e.g. minified JSON) defeating the line-based split and making
/// the "compact" fold as large as the raw output.
pub(crate) const SLICE_MAX_BYTES: usize = 8_000;

/// Result of a fold attempt.
#[derive(Debug, Clone)]
pub enum FoldOutcome {
    /// Output stayed under the gate: caller must use the original bytes
    /// untouched. `fold_output` never returns a copy in this case.
    Passthrough,
    /// Output exceeded the gate: full output is on disk at `Fold::path`,
    /// this carries a compact summary of it.
    Folded(Fold),
}

/// Compact summary of output that was folded to disk.
#[derive(Debug, Clone)]
pub struct Fold {
    /// First lines of output (byte-capped, UTF-8 safe).
    pub head: String,
    /// Last lines of output (byte-capped, UTF-8 safe), never overlapping
    /// `head`.
    pub tail: String,
    /// Total line count of the raw output.
    pub total_lines: usize,
    /// Path to the untruncated full output on disk.
    pub path: PathBuf,
    /// Byte length of the raw output.
    pub raw_bytes: usize,
    /// Byte length of `head` + `tail` combined.
    pub kept_bytes: usize,
}

impl Fold {
    /// THE single constructor for a `Fold`. Every producer goes through it —
    /// the in-memory `fold_output` and the streaming runner alike — so
    /// `head`/`tail`/`kept_bytes` semantics and what `render` prints can
    /// never drift between the two paths.
    ///
    /// `head_region` must contain at least the first `HEAD_LINES` lines or
    /// the first `SLICE_MAX_BYTES` bytes of the output, whichever ends
    /// first; `tail_region` must contain at least its last `TAIL_LINES`
    /// lines or last `SLICE_MAX_BYTES` bytes. Both may be the whole output
    /// (in-memory path) or bounded windows of it (streaming path) — the
    /// selection below reads no further than those guarantees.
    ///
    /// Regions are raw bytes: child output is arbitrary and may not be
    /// UTF-8. Only these small previews are lossily decoded; the persisted
    /// file and the passthrough path never are.
    pub(crate) fn from_regions(
        head_region: &[u8],
        tail_region: &[u8],
        total_lines: usize,
        raw_bytes: usize,
        path: PathBuf,
    ) -> Fold {
        let head_take = HEAD_LINES.min(total_lines);
        // Subtracting the head's share first is what keeps head and tail from
        // overlapping when the output has fewer than HEAD_LINES + TAIL_LINES
        // lines (e.g. one huge minified line).
        let tail_take = total_lines.saturating_sub(head_take).min(TAIL_LINES);

        let head = lossy_prefix(first_lines(head_region, head_take), SLICE_MAX_BYTES);
        let tail = lossy_suffix(last_lines(tail_region, tail_take), SLICE_MAX_BYTES);
        let kept_bytes = head.len() + tail.len();

        Fold {
            head,
            tail,
            total_lines,
            path,
            raw_bytes,
            kept_bytes,
        }
    }

    /// Render a compact human/agent-readable block: head, an omission
    /// marker with the total line count, tail, then the recovery
    /// pointer — the full-output path and a `tail` command to see what
    /// was omitted between the shown head and tail.
    pub fn render(&self) -> String {
        let head_lines = self.head.lines().count();
        let tail_lines = self.tail.lines().count();
        let omitted = self.total_lines.saturating_sub(head_lines + tail_lines);

        let mut out = String::new();
        out.push_str(&self.head);
        if !self.head.is_empty() && !self.head.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(&format!(
            "\n... {omitted} of {} lines omitted ...\n\n",
            self.total_lines
        ));
        out.push_str(&self.tail);
        if !self.tail.is_empty() && !self.tail.ends_with('\n') {
            out.push('\n');
        }
        out.push('\n');
        out.push_str(&format_hint(&self.path));
        out.push('\n');
        out.push_str(&format_tail_hint(&self.path, head_lines + 1));
        out
    }
}

/// Fold `raw` if its estimated token count (`ceil(len_bytes / 4)`) meets
/// or exceeds `cfg.threshold_tokens`: writes the full, untruncated output
/// to a private rotating file and returns a compact head/tail summary.
/// Below the threshold, or when `cfg.enabled` is false, returns
/// `Passthrough` and `raw` must be used byte-for-byte as-is.
pub fn fold_output(raw: &str, slug: &str, cfg: &FoldConfig) -> io::Result<FoldOutcome> {
    if !cfg.enabled {
        return Ok(FoldOutcome::Passthrough);
    }
    if estimate_tokens(raw) < cfg.threshold_tokens {
        return Ok(FoldOutcome::Passthrough);
    }

    let dir = resolve_fold_dir(cfg).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "no fold directory available (no data_local_dir on this platform)",
        )
    })?;
    let bytes = raw.as_bytes();
    let path = write_fold_file(bytes, slug, &dir, cfg.max_files)?;

    // Whole output as both regions: it is already in memory here, and
    // `from_regions` reads no further into either than it needs to.
    Ok(FoldOutcome::Folded(Fold::from_regions(
        bytes,
        bytes,
        count_lines(bytes),
        bytes.len(),
        path,
    )))
}

/// Total line count of `bytes`, defined as the newline count plus a final
/// unterminated line if there is one. Identical to `str::lines().count()`
/// for valid UTF-8, but computable from a running counter, which is what
/// the streaming runner needs — it counts `\n` per chunk and never
/// materializes a line.
pub(crate) fn count_lines(bytes: &[u8]) -> usize {
    total_lines_from(
        bytes.iter().filter(|b| **b == b'\n').count(),
        bytes.last().copied(),
    )
}

/// `count_lines` in incremental form: newlines seen so far plus the last
/// byte seen. The streaming runner keeps exactly these two values.
pub(crate) fn total_lines_from(newlines: usize, last_byte: Option<u8>) -> usize {
    match last_byte {
        Some(b'\n') | None => newlines,
        // Trailing bytes after the last newline are one more (unterminated) line.
        Some(_) => newlines + 1,
    }
}

/// First `n` lines of `bytes`, without the newline that terminates line `n`.
/// Returns everything when `bytes` holds fewer than `n` newlines — including
/// a trailing partial line, which is what a byte-bounded head window has.
fn first_lines(bytes: &[u8], n: usize) -> &[u8] {
    if n == 0 {
        return &[];
    }
    let mut seen = 0;
    for (i, b) in bytes.iter().enumerate() {
        if *b == b'\n' {
            seen += 1;
            if seen == n {
                return &bytes[..i];
            }
        }
    }
    bytes
}

/// Last `n` lines of `bytes`, without a trailing newline. One trailing
/// newline is dropped first so `"a\nb\n"` with `n == 1` yields `"b"`, not an
/// empty final line.
fn last_lines(bytes: &[u8], n: usize) -> &[u8] {
    if n == 0 {
        return &[];
    }
    let end = match bytes.last() {
        Some(b'\n') => bytes.len() - 1,
        _ => bytes.len(),
    };
    let body = &bytes[..end];
    let mut seen = 0;
    for i in (0..body.len()).rev() {
        if body[i] == b'\n' {
            seen += 1;
            if seen == n {
                return &body[i + 1..];
            }
        }
    }
    body
}

/// Decode at most `max_bytes` from the start of `bytes` for display.
///
/// A multi-byte char straddling the cap is dropped rather than turned into
/// U+FFFD, so a clean cut of valid UTF-8 stays clean. Genuinely invalid
/// bytes still decode lossily — the preview must never fail on binary
/// output — and the result is re-capped afterwards because each U+FFFD is
/// wider than the byte it replaces.
fn lossy_prefix(bytes: &[u8], max_bytes: usize) -> String {
    let slice = &bytes[..max_bytes.min(bytes.len())];
    let end = match std::str::from_utf8(slice) {
        Ok(_) => slice.len(),
        // error_len None means "valid so far, sequence cut off at the end".
        Err(e) if e.error_len().is_none() => e.valid_up_to(),
        Err(_) => slice.len(),
    };
    let text = String::from_utf8_lossy(&slice[..end]).into_owned();
    if text.len() <= max_bytes {
        return text;
    }
    take_prefix_bytes(&text, max_bytes)
}

/// Decode at most `max_bytes` from the end of `bytes` for display. Mirror of
/// `lossy_prefix`: leading continuation bytes left by the cut are dropped.
fn lossy_suffix(bytes: &[u8], max_bytes: usize) -> String {
    let start = bytes.len().saturating_sub(max_bytes);
    let mut slice = &bytes[start..];
    // A UTF-8 continuation byte is 0b10xxxxxx and can never start a char; at
    // most three of them precede the first real boundary. Applied even at
    // start == 0, because a byte-bounded tail window can itself begin
    // mid-char. Well-formed text is unaffected.
    let skip = slice
        .iter()
        .take(3)
        .take_while(|b| (**b & 0b1100_0000) == 0b1000_0000)
        .count();
    slice = &slice[skip..];
    let text = String::from_utf8_lossy(slice).into_owned();
    if text.len() <= max_bytes {
        return text;
    }
    take_suffix_bytes(&text, max_bytes)
}

/// Keep at most `max_bytes` from the start of `s`, cut on a UTF-8 char
/// boundary. Adapted from rtk's truncation-boundary logic in
/// `write_tee_file` (reference/rtk/src/core/tee.rs) — reused here only
/// for the in-memory summary, never for the persisted file.
fn take_prefix_bytes(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    // Walk DOWN to the nearest boundary at or below the cap. Taking every char
    // that starts below the cap and then adding its full width overshoots by up
    // to three bytes whenever a multi-byte char straddles the limit.
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

/// Keep at most `max_bytes` from the end of `s`, cut on a UTF-8 char
/// boundary.
fn take_suffix_bytes(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let min_start = s.len() - max_bytes;
    let boundary = s
        .char_indices()
        .find(|(i, _)| *i >= min_start)
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    s[boundary..].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_with_dir(dir: &std::path::Path) -> FoldConfig {
        FoldConfig {
            directory: Some(dir.to_path_buf()),
            ..FoldConfig::default()
        }
    }

    #[test]
    fn passthrough_below_threshold() {
        let tmpdir = tempfile::tempdir().unwrap();
        let cfg = FoldConfig {
            threshold_tokens: 1000,
            ..cfg_with_dir(tmpdir.path())
        };
        let raw = "small output\n";
        let outcome = fold_output(raw, "cmd", &cfg).unwrap();
        assert!(matches!(outcome, FoldOutcome::Passthrough));
    }

    #[test]
    fn passthrough_when_disabled_even_if_huge() {
        let tmpdir = tempfile::tempdir().unwrap();
        let cfg = FoldConfig {
            enabled: false,
            threshold_tokens: 10,
            ..cfg_with_dir(tmpdir.path())
        };
        let raw = "x".repeat(10_000);
        let outcome = fold_output(&raw, "cmd", &cfg).unwrap();
        assert!(matches!(outcome, FoldOutcome::Passthrough));
    }

    #[test]
    fn folds_at_or_above_threshold() {
        let tmpdir = tempfile::tempdir().unwrap();
        let cfg = FoldConfig {
            threshold_tokens: 100,
            ..cfg_with_dir(tmpdir.path())
        };
        // 100 tokens * 4 bytes/token = 400 bytes at the boundary.
        let raw = "x".repeat(400);
        let outcome = fold_output(&raw, "cmd", &cfg).unwrap();
        assert!(matches!(outcome, FoldOutcome::Folded(_)));
    }

    #[test]
    fn folded_file_on_disk_is_never_truncated() {
        let tmpdir = tempfile::tempdir().unwrap();
        let cfg = FoldConfig {
            threshold_tokens: 10,
            ..cfg_with_dir(tmpdir.path())
        };
        let raw: String = (0..5000).map(|i| format!("line {i}\n")).collect();
        let outcome = fold_output(&raw, "cmd", &cfg).unwrap();
        let FoldOutcome::Folded(fold) = outcome else {
            panic!("expected Folded");
        };
        let on_disk = std::fs::read_to_string(&fold.path).unwrap();
        assert_eq!(
            on_disk, raw,
            "persisted fold file must be the full, untruncated output"
        );
    }

    #[test]
    fn head_and_tail_never_overlap_on_small_line_count() {
        let tmpdir = tempfile::tempdir().unwrap();
        let cfg = FoldConfig {
            threshold_tokens: 10,
            ..cfg_with_dir(tmpdir.path())
        };
        // One giant line: huge byte size, but only 1 line total.
        let raw = "x".repeat(50_000);
        let outcome = fold_output(&raw, "cmd", &cfg).unwrap();
        let FoldOutcome::Folded(fold) = outcome else {
            panic!("expected Folded");
        };
        assert_eq!(fold.total_lines, 1);
        assert!(
            fold.tail.is_empty(),
            "single-line input has nothing left for tail"
        );
        assert!(fold.head.len() <= SLICE_MAX_BYTES);
    }

    #[test]
    fn kept_bytes_much_smaller_than_raw_for_large_output() {
        let tmpdir = tempfile::tempdir().unwrap();
        let cfg = FoldConfig {
            threshold_tokens: 10,
            ..cfg_with_dir(tmpdir.path())
        };
        let raw: String = (0..10_000).map(|i| format!("line {i}\n")).collect();
        let outcome = fold_output(&raw, "cmd", &cfg).unwrap();
        let FoldOutcome::Folded(fold) = outcome else {
            panic!("expected Folded");
        };
        assert!(fold.kept_bytes < fold.raw_bytes / 10);
        assert_eq!(fold.raw_bytes, raw.len());
    }

    #[test]
    fn render_contains_totals_and_recovery_pointer() {
        let tmpdir = tempfile::tempdir().unwrap();
        let cfg = FoldConfig {
            threshold_tokens: 10,
            ..cfg_with_dir(tmpdir.path())
        };
        let raw: String = (0..500).map(|i| format!("line {i}\n")).collect();
        let outcome = fold_output(&raw, "my_cmd", &cfg).unwrap();
        let FoldOutcome::Folded(fold) = outcome else {
            panic!("expected Folded");
        };
        let rendered = fold.render();
        assert!(rendered.contains("line 0"), "must show head");
        assert!(rendered.contains("line 499"), "must show tail");
        assert!(rendered.contains("500"), "must state total line count");
        assert!(rendered.contains("[full output: "));
        assert!(rendered.contains("[see remaining: tail -n +"));
    }

    #[test]
    fn take_prefix_and_suffix_bytes_respect_utf8_boundaries() {
        let japanese = "\u{6F22}".repeat(333); // 999 bytes, 3-byte chars
        let prefix = take_prefix_bytes(&japanese, 998);
        assert!(prefix.len() <= 998);
        assert!(japanese.starts_with(&prefix));

        let suffix = take_suffix_bytes(&japanese, 998);
        assert!(suffix.len() <= 998);
        assert!(japanese.ends_with(&suffix));
    }

    #[test]
    fn count_lines_matches_str_lines_count() {
        for raw in ["", "a", "a\n", "a\nb", "a\nb\n", "a\n\n", "\n", "\n\n"] {
            assert_eq!(
                count_lines(raw.as_bytes()),
                raw.lines().count(),
                "count_lines disagrees with str::lines on {raw:?}"
            );
        }
    }

    #[test]
    fn first_and_last_lines_select_without_terminators() {
        let raw = b"a\nb\nc\n";
        assert_eq!(first_lines(raw, 0), b"");
        assert_eq!(first_lines(raw, 2), b"a\nb");
        assert_eq!(
            first_lines(raw, 9),
            raw.as_slice(),
            "fewer lines than asked: take all"
        );
        assert_eq!(last_lines(raw, 0), b"");
        assert_eq!(last_lines(raw, 1), b"c");
        assert_eq!(last_lines(raw, 2), b"b\nc");
        assert_eq!(last_lines(b"a\nb", 1), b"b");
    }

    #[test]
    fn lossy_slices_never_exceed_the_cap_on_invalid_utf8() {
        // Every byte is invalid UTF-8 and expands to a 3-byte U+FFFD.
        let junk = vec![0xffu8; 100];
        assert!(lossy_prefix(&junk, 12).len() <= 12);
        assert!(lossy_suffix(&junk, 12).len() <= 12);
    }

    #[test]
    fn lossy_prefix_drops_a_char_cut_by_the_cap() {
        let text = "ab\u{6F22}cd"; // 'a','b', 3-byte char, 'c','d'
        // Cap 3 lands inside the multi-byte char: drop it, do not pad U+FFFD.
        assert_eq!(lossy_prefix(text.as_bytes(), 3), "ab");
    }

    #[test]
    fn streaming_and_in_memory_regions_produce_the_same_fold() {
        // The single-constructor guarantee: bounded windows (what the runner
        // keeps) and the whole buffer must yield an identical Fold.
        let raw: String = (0..500).map(|i| format!("line {i}\n")).collect();
        let bytes = raw.as_bytes();
        let total = count_lines(bytes);
        let path = PathBuf::from("/tmp/x.log");

        let whole = Fold::from_regions(bytes, bytes, total, bytes.len(), path.clone());
        let head_window = &bytes[..SLICE_MAX_BYTES.min(bytes.len())];
        let tail_window = &bytes[bytes.len().saturating_sub(2 * SLICE_MAX_BYTES)..];
        let windowed =
            Fold::from_regions(head_window, tail_window, total, bytes.len(), path.clone());

        assert_eq!(whole.head, windowed.head);
        assert_eq!(whole.tail, windowed.tail);
        assert_eq!(whole.kept_bytes, windowed.kept_bytes);
        assert_eq!(whole.render(), windowed.render());
    }

    #[test]
    fn slug_is_used_in_persisted_filename() {
        let tmpdir = tempfile::tempdir().unwrap();
        let cfg = FoldConfig {
            threshold_tokens: 10,
            ..cfg_with_dir(tmpdir.path())
        };
        let raw = "x".repeat(1000);
        let outcome = fold_output(&raw, "my-special-cmd", &cfg).unwrap();
        let FoldOutcome::Folded(fold) = outcome else {
            panic!("expected Folded");
        };
        assert!(fold.path.to_string_lossy().contains("my-special-cmd"));
    }
}
