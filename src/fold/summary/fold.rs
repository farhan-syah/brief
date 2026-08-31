//! The fold itself: new code — rtk never truncates output back to its
//! caller, it only writes a recovery hint alongside the untouched output.
//! Here, output past the token gate is replaced by a compact head/tail
//! summary; the full output is always on disk, untruncated, via `write`.

use std::io;
use std::path::PathBuf;

use crate::thousands::with_thousands_separator;

use super::super::config::FoldConfig;
use super::super::paths::{format_full_output_hint, resolve_fold_dir};
use super::super::tokens::estimate_tokens;
use super::super::write::write_fold_file;
use super::lines::{count_lines, first_lines, last_lines};
use super::preview::{SLICE_MAX_BYTES, lossy_prefix, lossy_suffix};

/// Lines kept from the start of output when folding.
pub(crate) const HEAD_LINES: usize = 50;
/// Lines kept from the end of output when folding.
pub(crate) const TAIL_LINES: usize = 50;

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
            "\n... {} of {} lines omitted ...\n\n",
            with_thousands_separator(omitted),
            with_thousands_separator(self.total_lines)
        ));
        out.push_str(&self.tail);
        if !self.tail.is_empty() && !self.tail.ends_with('\n') {
            out.push('\n');
        }
        out.push('\n');
        out.push_str(&format_full_output_hint(&self.path, head_lines + 1));
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

#[cfg(test)]
mod tests {
    use crate::fold::paths::display_shell_path;

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
        assert!(rendered.contains("[full output: brief tail -n +"));
        let displayed_path = display_shell_path(&fold.path);
        assert_eq!(
            rendered.matches(&displayed_path).count(),
            1,
            "the fold path must be printed exactly once, not repeated across two hint lines"
        );
    }

    #[test]
    fn render_adds_thousands_separators_to_omission_marker() {
        let tmpdir = tempfile::tempdir().unwrap();
        let cfg = FoldConfig {
            threshold_tokens: 10,
            ..cfg_with_dir(tmpdir.path())
        };
        let raw: String = (0..4231).map(|i| format!("line {i}\n")).collect();
        let outcome = fold_output(&raw, "my_cmd", &cfg).unwrap();
        let FoldOutcome::Folded(fold) = outcome else {
            panic!("expected Folded");
        };
        let rendered = fold.render();
        assert!(
            rendered.contains("4,131 of 4,231 lines omitted"),
            "omission marker must use thousands separators: {rendered}"
        );
    }

    #[test]
    fn with_thousands_separator_shape() {
        assert_eq!(with_thousands_separator(0), "0");
        assert_eq!(with_thousands_separator(51), "51");
        assert_eq!(with_thousands_separator(999), "999");
        assert_eq!(with_thousands_separator(1000), "1,000");
        assert_eq!(with_thousands_separator(4131), "4,131");
        assert_eq!(with_thousands_separator(1234567), "1,234,567");
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
