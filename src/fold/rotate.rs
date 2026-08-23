//! Adapted from rtk's `core::tee::cleanup_old_files`.
//! Source: reference/rtk/src/core/tee.rs
//!
//! Deviation from rtk: `max_files` is floored to 1 before computing how
//! many files to remove, so a caller passing `max_files: 0` can never
//! delete the fold file that was just written — rotation must never
//! destroy the newest file.

use std::path::Path;

/// Rotate old fold files: keep only the newest `max_files` (by filename,
/// which sorts chronologically — see the `{epoch}_{slug}.log` format),
/// delete the rest.
pub(crate) fn cleanup_old_files(dir: &Path, max_files: usize) {
    let keep = max_files.max(1);

    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "log"))
        .collect();

    if entries.len() <= keep {
        return;
    }

    // Sort by filename (which starts with epoch timestamp = chronological).
    entries.sort_by_key(|e| e.file_name());

    let to_remove = entries.len() - keep;
    for entry in entries.iter().take(to_remove) {
        let _ = std::fs::remove_file(entry.path());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn keeps_newest_max_files() {
        let tmpdir = tempfile::tempdir().unwrap();
        let dir = tmpdir.path();

        for i in 0..25 {
            let filename = format!("{:010}_test.log", 1_000_000 + i);
            fs::write(dir.join(&filename), "content").unwrap();
        }

        cleanup_old_files(dir, 20);

        let remaining: Vec<_> = fs::read_dir(dir).unwrap().filter_map(|e| e.ok()).collect();
        assert_eq!(remaining.len(), 20);

        for i in 0..5 {
            let filename = format!("{:010}_test.log", 1_000_000 + i);
            assert!(!dir.join(&filename).exists());
        }
        for i in 5..25 {
            let filename = format!("{:010}_test.log", 1_000_000 + i);
            assert!(dir.join(&filename).exists());
        }
    }

    #[test]
    fn ignores_non_log_files() {
        let tmpdir = tempfile::tempdir().unwrap();
        let dir = tmpdir.path();
        fs::write(dir.join("notes.txt"), "keep me").unwrap();
        for i in 0..5 {
            fs::write(dir.join(format!("{:010}_x.log", i)), "c").unwrap();
        }

        cleanup_old_files(dir, 1);

        assert!(dir.join("notes.txt").exists());
    }

    #[test]
    fn max_files_zero_never_deletes_the_newest_file() {
        let tmpdir = tempfile::tempdir().unwrap();
        let dir = tmpdir.path();
        for i in 0..5 {
            fs::write(dir.join(format!("{:010}_x.log", i)), "c").unwrap();
        }

        cleanup_old_files(dir, 0);

        // Floored to keep(1): the newest file (highest epoch prefix) survives.
        assert!(dir.join("0000000004_x.log").exists());
        let remaining: Vec<_> = fs::read_dir(dir).unwrap().filter_map(|e| e.ok()).collect();
        assert_eq!(remaining.len(), 1);
    }
}
