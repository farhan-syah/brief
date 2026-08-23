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
const HEAD_LINES: usize = 50;
/// Lines kept from the end of output when folding.
const TAIL_LINES: usize = 50;
/// Byte cap per kept slice (head/tail), guarding against a single huge
/// line (e.g. minified JSON) defeating the line-based split and making
/// the "compact" fold as large as the raw output.
const SLICE_MAX_BYTES: usize = 8_000;

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
    let path = write_fold_file(raw, slug, &dir, cfg.max_files)?;

    let lines: Vec<&str> = raw.lines().collect();
    let total_lines = lines.len();
    let head_take = HEAD_LINES.min(total_lines);
    let tail_start = total_lines.saturating_sub(TAIL_LINES).max(head_take);

    let head = take_prefix_bytes(&lines[..head_take].join("\n"), SLICE_MAX_BYTES);
    let tail = take_suffix_bytes(&lines[tail_start..].join("\n"), SLICE_MAX_BYTES);
    let kept_bytes = head.len() + tail.len();

    Ok(FoldOutcome::Folded(Fold {
        head,
        tail,
        total_lines,
        path,
        raw_bytes: raw.len(),
        kept_bytes,
    }))
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
