//! Ported from rtk's `core::utils` (create_private_dir / open_private /
//! set_owner_only only — the rest of utils.rs is out of scope for sigfold).
//! Source: reference/rtk/src/core/utils.rs

use std::fs;
use std::io;
use std::path::Path;

/// Create a directory (and its parents) restricted to owner-only access
/// (0700 on Unix).
pub(crate) fn create_private_dir(path: &Path) -> io::Result<()> {
    fs::create_dir_all(path)?;
    set_owner_only(path, 0o700);
    Ok(())
}

/// Open a file owner-only (0600 on Unix), applied at creation so content
/// is never briefly readable under a permissive umask. `mode` is ignored
/// for a file that already exists, so an older one is still tightened
/// afterwards.
pub(crate) fn open_private(opts: &mut fs::OpenOptions, path: &Path) -> io::Result<fs::File> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let file = opts.open(path)?;
    set_owner_only(path, 0o600);
    Ok(file)
}

#[cfg(unix)]
fn set_owner_only(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(mode));
}

#[cfg(not(unix))]
fn set_owner_only(_path: &Path, _mode: u32) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(unix)]
    fn create_private_dir_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let tmpdir = tempfile::tempdir().unwrap();
        let dir = tmpdir.path().join("private");
        create_private_dir(&dir).unwrap();

        let mode = fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
    }

    #[test]
    #[cfg(unix)]
    fn open_private_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let tmpdir = tempfile::tempdir().unwrap();
        let path = tmpdir.path().join("secret.log");
        let file = open_private(fs::OpenOptions::new().write(true).create(true), &path).unwrap();
        drop(file);

        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    #[cfg(unix)]
    #[allow(unsafe_code)]
    fn open_private_owner_only_under_permissive_umask() {
        use std::os::unix::fs::PermissionsExt;

        // nosemgrep: unsafe-block
        let previous = unsafe { libc::umask(0o000) };
        let tmpdir = tempfile::tempdir().unwrap();
        let path = tmpdir.path().join("secret.log");
        let file = open_private(fs::OpenOptions::new().write(true).create(true), &path);
        // nosemgrep: unsafe-block
        unsafe { libc::umask(previous) };

        let file = file.expect("file opened");
        drop(file);
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "umask 000 must not widen the fold file");
    }
}
