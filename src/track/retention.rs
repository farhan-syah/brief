//! Compaction: keep the newest rows, bounded by BOTH a retention window and
//! a byte budget, whichever binds first. Rows are appended in time order,
//! so "newest" is a suffix of the file — no sorting required.
//!
//! The byte budget is what actually bounds the file's size; the retention
//! window only ever drops more rows than the budget alone would. Together
//! with `append`'s trigger/target gap this hard-bounds the tracking file at
//! `compact_trigger_bytes` (40 MiB by default) no matter how heavily the
//! tool is used. A time window alone does not bound anything: usage volume
//! sets how much lands inside it.
//!
//! The file is rewritten to a temp file in the same directory and renamed
//! over the original, so a crash mid-compaction never truncates the audit
//! trail.

use std::fs;
use std::io::{self, Write};
use std::path::Path;

use crate::private_fs::open_private;

use super::record::now_ms;

const MS_PER_DAY: u64 = 24 * 60 * 60 * 1000;

/// Rewrite `path`, keeping the newest rows subject to both `retention_days`
/// and `target_bytes`. Best-effort: every error here is swallowed by the
/// caller, tracking must never fail the command it is observing.
///
/// The file is read as raw bytes, never decoded as UTF-8. A line that is
/// malformed — truncated, or not valid UTF-8 at all — is dropped like any
/// other unparseable row. Decoding the whole file instead would let one
/// corrupt byte fail every future compaction, and a compaction that can
/// never succeed is a tracking file that grows without bound.
pub(crate) fn compact(path: &Path, retention_days: u64, target_bytes: u64) -> io::Result<()> {
    let contents = fs::read(path)?;
    let now = now_ms();
    let cutoff = now.saturating_sub(retention_days as u128 * MS_PER_DAY as u128);

    // Walk from the newest line (end of file) backward, keeping a
    // contiguous suffix: skip anything malformed or past the retention
    // window, and stop entirely once the byte budget for kept lines would
    // be exceeded — everything older than that point is dropped too.
    let mut kept_rev: Vec<&[u8]> = Vec::new();
    let mut kept_bytes: u64 = 0;
    for line in contents.split(|&b| b == b'\n').rev() {
        if line.is_empty() {
            continue; // the trailing newline, or a blank line
        }
        let Some(ts) = line_ts_ms(line) else {
            continue; // malformed: drop this one line, keep scanning
        };
        if ts < cutoff {
            continue;
        }
        let line_bytes = line.len() as u64 + 1; // +1 for the newline
        if kept_bytes + line_bytes > target_bytes {
            break; // budget exhausted: this and everything older is dropped
        }
        kept_rev.push(line);
        kept_bytes += line_bytes;
    }

    let mut kept: Vec<u8> = Vec::with_capacity(kept_bytes as usize);
    for line in kept_rev.into_iter().rev() {
        kept.extend_from_slice(line);
        kept.push(b'\n');
    }

    let tmp_path = tmp_path_for(path);
    let mut tmp = open_private(
        fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true),
        &tmp_path,
    )?;
    tmp.write_all(&kept)?;
    tmp.flush()?;
    drop(tmp);
    fs::rename(&tmp_path, path)?;
    Ok(())
}

/// Temp file path in the same directory as `path`, so the final `rename` is
/// a same-filesystem, atomic operation.
fn tmp_path_for(path: &Path) -> std::path::PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".tmp");
    path.with_file_name(name)
}

/// Extract `ts_ms` from a raw JSONL line without a JSON parser: find
/// `"ts_ms":` and read the digits that follow it. Operates on bytes, so a
/// line that is not valid UTF-8 is merely unparseable rather than an error.
///
/// This substring search is safe ONLY because `record::InvocationRecord::
/// serialize` always emits `ts_ms` as the first field, so the first match
/// is always the real one — `args` (the one field with arbitrary user
/// text) can never contain a `"ts_ms":` byte sequence that this function
/// would see before the real one. `report::parse::parse_line` cannot make
/// this assumption (it also needs `program`, `cwd`, etc., not just the
/// first field), which is why it uses a full string-aware scanner instead.
fn line_ts_ms(line: &[u8]) -> Option<u128> {
    let key = b"\"ts_ms\":";
    let idx = line.windows(key.len()).position(|w| w == key)?;
    let rest = &line[idx + key.len()..];
    let end = rest
        .iter()
        .position(|b| !b.is_ascii_digit())
        .unwrap_or(rest.len());
    if end == 0 {
        return None;
    }
    std::str::from_utf8(&rest[..end]).ok()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(ts_ms: u128, tag: &str) -> String {
        format!("{{\"ts_ms\":{ts_ms},\"program\":\"{tag}\"}}\n")
    }

    #[test]
    fn drops_rows_older_than_retention_and_keeps_the_rest() {
        let tmpdir = tempfile::tempdir().unwrap();
        let path = tmpdir.path().join("tracking.jsonl");
        let now = now_ms();
        let old = now.saturating_sub(200 * MS_PER_DAY as u128);
        let recent = now.saturating_sub(MS_PER_DAY as u128);
        fs::write(
            &path,
            format!("{}{}", row(old, "old"), row(recent, "recent")),
        )
        .unwrap();

        compact(&path, 90, 32 * 1024 * 1024).unwrap();

        let contents = fs::read_to_string(&path).unwrap();
        assert_eq!(contents.lines().count(), 1);
        assert!(contents.contains("recent"));
        assert!(!contents.contains("\"old\""));
    }

    #[test]
    fn keeps_newest_rows_in_order_when_over_the_byte_budget() {
        let tmpdir = tempfile::tempdir().unwrap();
        let path = tmpdir.path().join("tracking.jsonl");
        let now = now_ms();

        // 10 rows, each ~50 bytes, oldest first (append order).
        let mut contents = String::new();
        for i in 0..10 {
            contents.push_str(&row(now - (10 - i), &format!("row{i}")));
        }
        fs::write(&path, &contents).unwrap();

        // Budget only large enough for the newest 3 rows.
        let row_len = row(now, "row0").len() as u64;
        compact(&path, 90, row_len * 3).unwrap();

        let kept: Vec<String> = fs::read_to_string(&path)
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect();
        assert_eq!(kept.len(), 3);
        assert!(kept[0].contains("row7"));
        assert!(kept[1].contains("row8"));
        assert!(kept[2].contains("row9"), "newest row must survive");
    }

    #[test]
    fn old_row_dropped_even_when_under_the_byte_budget() {
        let tmpdir = tempfile::tempdir().unwrap();
        let path = tmpdir.path().join("tracking.jsonl");
        let now = now_ms();
        let old = now.saturating_sub(200 * MS_PER_DAY as u128);
        fs::write(&path, row(old, "ancient")).unwrap();

        // Byte budget is huge — only the retention window should apply.
        compact(&path, 90, 32 * 1024 * 1024).unwrap();

        let contents = fs::read_to_string(&path).unwrap();
        assert!(
            contents.is_empty(),
            "an old row must be dropped even when there is byte budget to spare"
        );
    }

    #[test]
    fn malformed_line_in_the_middle_is_dropped_not_fatal() {
        let tmpdir = tempfile::tempdir().unwrap();
        let path = tmpdir.path().join("tracking.jsonl");
        let now = now_ms();
        let contents = format!(
            "{}not json at all, no ts_ms field\n{}",
            row(now, "before"),
            row(now, "after")
        );
        fs::write(&path, &contents).unwrap();

        compact(&path, 90, 32 * 1024 * 1024).unwrap();

        let kept = fs::read_to_string(&path).unwrap();
        assert!(kept.contains("before"));
        assert!(kept.contains("after"));
        assert!(!kept.contains("not json"));
        assert_eq!(kept.lines().count(), 2);
    }

    #[test]
    fn invalid_utf8_line_is_dropped_and_does_not_fail_compaction() {
        // A compaction that can never succeed is a tracking file that grows
        // without bound: `append` swallows the failure and writes anyway,
        // so one corrupt byte would silently retire the size guarantee.
        let tmpdir = tempfile::tempdir().unwrap();
        let path = tmpdir.path().join("tracking.jsonl");
        let now = now_ms();
        let mut contents: Vec<u8> = Vec::new();
        contents.extend_from_slice(row(now, "before").as_bytes());
        contents.extend_from_slice(&[0xff, 0xfe, 0x80]); // never valid UTF-8
        contents.push(b'\n');
        contents.extend_from_slice(row(now, "after").as_bytes());
        fs::write(&path, &contents).unwrap();

        compact(&path, 90, 32 * 1024 * 1024).unwrap();

        let kept = fs::read(&path).unwrap();
        assert!(!kept.contains(&0xff), "the corrupt line must be dropped");
        let kept = String::from_utf8(kept).expect("survivors are valid UTF-8");
        assert!(kept.contains("before"));
        assert!(kept.contains("after"));
        assert_eq!(kept.lines().count(), 2);
    }

    #[test]
    fn rewrite_is_atomic_via_rename_and_leaves_no_tmp_file() {
        let tmpdir = tempfile::tempdir().unwrap();
        let path = tmpdir.path().join("tracking.jsonl");
        fs::write(&path, row(now_ms(), "x")).unwrap();

        compact(&path, 90, 32 * 1024 * 1024).unwrap();

        assert!(path.exists());
        assert!(!tmp_path_for(&path).exists());
    }

    #[test]
    #[cfg(unix)]
    fn compacted_file_stays_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let tmpdir = tempfile::tempdir().unwrap();
        let path = tmpdir.path().join("tracking.jsonl");
        fs::write(&path, row(now_ms(), "x")).unwrap();

        compact(&path, 90, 32 * 1024 * 1024).unwrap();

        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
