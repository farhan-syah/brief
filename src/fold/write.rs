//! Adapted from rtk's `core::tee::write_tee_file` / `create_tee_dir`.
//! Source: reference/rtk/src/core/tee.rs
//!
//! Deviation from rtk: the truncation-at-`max_file_size` branch and the
//! `max_file_size` parameter are dropped entirely. The persisted file is
//! never truncated — rtk's truncation made its "full output" recovery
//! hint false for exactly the large outputs that matter most.

use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::private_fs::{create_private_dir, open_private};
use super::rotate::cleanup_old_files;
use super::slug::sanitize_slug;

/// Creates the parent as its own step, otherwise `create_dir_all` leaves
/// the data root at the umask as an intermediate.
fn create_fold_dir(fold_dir: &Path) -> io::Result<()> {
    if let Some(parent) = fold_dir.parent() {
        let _ = create_private_dir(parent);
    }
    create_private_dir(fold_dir)
}

/// Create the fold directory and open an empty, owner-only fold file in it,
/// returning the handle and its path.
///
/// Split out of `write_fold_file` for the streaming runner, which must hold
/// the destination handle *before* it has the content: it opens the file the
/// moment a stream crosses the token gate and then writes every later chunk
/// straight through, so the full output never has to fit in memory.
///
/// Rotation is deliberately NOT done here — `cleanup_old_files` deletes the
/// oldest files, so it is only safe once the new file has been written. The
/// caller that finishes the write owns that call (and therefore `max_files`).
pub(crate) fn open_fold_file(command_slug: &str, fold_dir: &Path) -> io::Result<(File, PathBuf)> {
    create_fold_dir(fold_dir)?;

    let slug = sanitize_slug(command_slug);
    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_secs();
    let filename = format!("{epoch}_{slug}.log");
    let filepath = fold_dir.join(filename);

    let file = open_private(
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true),
        &filepath,
    )?;

    Ok((file, filepath))
}

/// Write the full, untruncated `raw` output to a fold file in `fold_dir`,
/// then rotate old files. Returns the written file's path.
///
/// Takes bytes, not `&str`: child output is arbitrary bytes and the
/// persisted file must be byte-for-byte what the child produced.
pub(crate) fn write_fold_file(
    raw: &[u8],
    command_slug: &str,
    fold_dir: &Path,
    max_files: usize,
) -> io::Result<PathBuf> {
    let (mut file, filepath) = open_fold_file(command_slug, fold_dir)?;
    file.write_all(raw)?;

    cleanup_old_files(fold_dir, max_files);

    Ok(filepath)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn writes_full_content_untruncated() {
        let tmpdir = tempfile::tempdir().unwrap();
        let content = "error: test failed\n".repeat(50_000); // ~1MB, past rtk's old cap
        let path = write_fold_file(content.as_bytes(), "cargo_test", tmpdir.path(), 20).unwrap();

        let written = fs::read_to_string(&path).unwrap();
        assert_eq!(written, content, "fold file must never be truncated");
    }

    #[test]
    fn filename_matches_epoch_underscore_slug_format() {
        // Rotation sorts by filename and relies on this exact shape.
        let tmpdir = tempfile::tempdir().unwrap();
        let path = write_fold_file(b"hello", "my-cmd", tmpdir.path(), 20).unwrap();
        let name = path.file_name().unwrap().to_str().unwrap();

        let (epoch_part, rest) = name.split_once('_').expect("epoch_slug.log shape");
        assert!(
            epoch_part.chars().all(|c| c.is_ascii_digit()),
            "prefix before first underscore must be all-digit epoch seconds, got '{}'",
            epoch_part
        );
        assert!(rest.ends_with(".log"));
        assert!(name.contains("my-cmd"));
    }

    #[test]
    #[cfg(unix)]
    fn fold_file_and_dir_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let tmpdir = tempfile::tempdir().unwrap();
        let fold_dir = tmpdir.path().join("folds");
        let path = write_fold_file(b"secret output\n", "grep", &fold_dir, 20).unwrap();

        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path), 0o600, "fold file must be owner-only");
        assert_eq!(mode(&fold_dir), 0o700, "fold dir must be owner-only");
    }

    #[test]
    fn rotates_after_write() {
        let tmpdir = tempfile::tempdir().unwrap();
        let dir = tmpdir.path();
        for i in 0..25 {
            fs::write(dir.join(format!("{:010}_old.log", i)), "c").unwrap();
        }

        let path = write_fold_file(b"newest", "slug", dir, 20).unwrap();

        assert!(path.exists(), "the file just written must survive rotation");
        let remaining: Vec<_> = fs::read_dir(dir).unwrap().filter_map(|e| e.ok()).collect();
        assert_eq!(remaining.len(), 20);
    }
}
