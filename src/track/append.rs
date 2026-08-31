//! Append one invocation record to the tracking file, compacting it first
//! once it has grown past `cfg.compact_trigger_bytes`.
//!
//! Tracking is best-effort and always last: `append` returns a `Result`
//! only so the single call site (`crate::runner::spawn`) can choose to
//! swallow it with `let _ = ...` — it must never delay, alter, or fail the
//! command being run, and never touch stdout/stderr.

use std::fs;
use std::io::{self, Write};

use crate::private_fs::{create_private_dir, open_private};

use super::config::TrackConfig;
use super::paths::resolve_track_path;
use super::record::InvocationRecord;
use super::retention::compact;

/// Append `record` as one JSONL line to the tracking file resolved from
/// `cfg`. A no-op when tracking is disabled.
pub(crate) fn append(record: &InvocationRecord, cfg: &TrackConfig) -> io::Result<()> {
    if !cfg.enabled {
        return Ok(());
    }
    let path = resolve_track_path(cfg).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "no tracking file location available (no data_local_dir on this platform)",
        )
    })?;
    if let Some(parent) = path.parent() {
        create_private_dir(parent)?;
    }

    // One cheap stat; compaction only runs once the file has actually
    // crossed the trigger. Below it, appending is a pure O(1) write with no
    // read — this is the common path, not an optimization on top of always
    // compacting. A compaction failure (e.g. a malformed existing file)
    // must not block the append that follows it.
    if let Ok(meta) = fs::metadata(&path)
        && meta.len() >= cfg.compact_trigger_bytes
    {
        let _ = compact(&path, cfg.retention_days, cfg.compact_target_bytes);
    }

    let line = record.to_line();
    let mut file = open_private(fs::OpenOptions::new().create(true).append(true), &path)?;
    // A single write_all of one complete, newline-terminated buffer — never
    // two writes — is what keeps this append atomic under POSIX's
    // O_APPEND/PIPE_BUF guarantee against a concurrent ogt invocation.
    file.write_all(line.as_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record_with_tag(tag: &str) -> InvocationRecord {
        InvocationRecord {
            ts_ms: super::super::record::now_ms(),
            program: tag.to_string(),
            args: "-r foo .".to_string(),
            cwd: Some("/home/user".to_string()),
            exit_code: 0,
            exec_time_ms: 5,
            stdout_raw_bytes: Some(100),
            stdout_kept_bytes: Some(100),
            stdout_folded: false,
            stdout_path: None,
            stderr_raw_bytes: Some(0),
            stderr_kept_bytes: Some(0),
            stderr_folded: false,
            stderr_path: None,
            reads_fold: false,
            captured: true,
        }
    }

    fn record() -> InvocationRecord {
        record_with_tag("grep")
    }

    fn cfg_with_path(path: &std::path::Path) -> TrackConfig {
        TrackConfig {
            path: Some(path.to_path_buf()),
            ..TrackConfig::default()
        }
    }

    #[test]
    fn appends_one_line_creating_parent_dir() {
        let tmpdir = tempfile::tempdir().unwrap();
        let path = tmpdir.path().join("nested").join("tracking.jsonl");
        let cfg = cfg_with_path(&path);

        append(&record(), &cfg).unwrap();

        let contents = fs::read_to_string(&path).unwrap();
        assert_eq!(contents.lines().count(), 1);
        assert!(contents.contains("\"program\":\"grep\""));
    }

    #[test]
    fn second_append_adds_a_second_line() {
        let tmpdir = tempfile::tempdir().unwrap();
        let path = tmpdir.path().join("tracking.jsonl");
        let cfg = cfg_with_path(&path);

        append(&record(), &cfg).unwrap();
        append(&record(), &cfg).unwrap();

        let contents = fs::read_to_string(&path).unwrap();
        assert_eq!(contents.lines().count(), 2);
    }

    #[test]
    fn disabled_is_a_no_op() {
        let tmpdir = tempfile::tempdir().unwrap();
        let path = tmpdir.path().join("tracking.jsonl");
        let cfg = TrackConfig {
            enabled: false,
            ..cfg_with_path(&path)
        };

        append(&record(), &cfg).unwrap();

        assert!(!path.exists());
    }

    #[test]
    #[cfg(unix)]
    fn tracking_file_and_dir_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let tmpdir = tempfile::tempdir().unwrap();
        let path = tmpdir.path().join("nested").join("tracking.jsonl");
        let cfg = cfg_with_path(&path);

        append(&record(), &cfg).unwrap();

        let mode = |p: &std::path::Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(path.parent().unwrap()), 0o700);
    }

    /// A file below the trigger is appended to and never rewritten —
    /// checked by content, not just size, so a false pass from an
    /// accidental no-op compaction can't hide behind a size assertion.
    #[test]
    fn below_trigger_is_a_pure_append_original_line_survives_untouched() {
        let tmpdir = tempfile::tempdir().unwrap();
        let path = tmpdir.path().join("tracking.jsonl");
        let cfg = TrackConfig {
            compact_trigger_bytes: 1024 * 1024,
            compact_target_bytes: 512 * 1024,
            ..cfg_with_path(&path)
        };

        append(&record_with_tag("first"), &cfg).unwrap();
        let after_first = fs::read_to_string(&path).unwrap();

        append(&record_with_tag("second"), &cfg).unwrap();
        let after_second = fs::read_to_string(&path).unwrap();

        assert!(
            after_second.starts_with(&after_first),
            "the original line must be byte-identical and still present, not rewritten"
        );
        assert!(after_second.contains("\"program\":\"second\""));
    }

    #[test]
    fn crossing_the_trigger_compacts_to_at_or_below_the_target() {
        let tmpdir = tempfile::tempdir().unwrap();
        let path = tmpdir.path().join("tracking.jsonl");
        let trigger = 8 * 1024;
        let target = 4 * 1024;
        let cfg = TrackConfig {
            compact_trigger_bytes: trigger,
            compact_target_bytes: target,
            ..cfg_with_path(&path)
        };

        // Seed a file already past the trigger, all rows well within
        // retention, oldest first (append order).
        let now = super::super::record::now_ms();
        let mut seed = String::new();
        let mut n = 0;
        while (seed.len() as u64) < trigger {
            seed.push_str(&format!(
                "{{\"ts_ms\":{},\"program\":\"seed{n}\"}}\n",
                now - (100_000 - n as u128)
            ));
            n += 1;
        }
        fs::write(&path, &seed).unwrap();

        append(&record_with_tag("newest"), &cfg).unwrap();

        let contents = fs::read_to_string(&path).unwrap();
        assert!(
            contents.len() as u64 <= target + 4096,
            "compacted file ({} bytes) must land at or near the target ({target} bytes), \
             plus the one row just appended",
            contents.len()
        );
        assert!(
            contents
                .lines()
                .last()
                .is_some_and(|l| l.contains("\"newest\"")),
            "the just-appended row must survive"
        );
        assert!(
            !contents.contains("\"seed0\""),
            "the oldest seeded rows must have been dropped, not the newest"
        );
    }

    #[test]
    fn repeated_appends_across_the_trigger_stay_bounded() {
        let tmpdir = tempfile::tempdir().unwrap();
        let path = tmpdir.path().join("tracking.jsonl");
        let trigger = 8 * 1024u64;
        let target = 4 * 1024u64;
        let cfg = TrackConfig {
            compact_trigger_bytes: trigger,
            compact_target_bytes: target,
            ..cfg_with_path(&path)
        };

        let mut max_line_bytes: u64 = 0;
        for i in 0..400 {
            let rec = record_with_tag(&format!("run{i}"));
            max_line_bytes = max_line_bytes.max(rec.to_line().len() as u64);
            append(&rec, &cfg).unwrap();

            let size = fs::metadata(&path).unwrap().len();
            assert!(
                size <= trigger + max_line_bytes,
                "file grew to {size} bytes on append {i}, past trigger {trigger} plus one line"
            );
        }
    }
}
