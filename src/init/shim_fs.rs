//! Filesystem side of `ogt init --shims`: writing and removing the
//! PATH-shim scripts `init::shims` renders. No shape/template logic here —
//! that's `shims`, kept pure and filesystem-free — mirroring `fs_ops`'s
//! split from `settings_edit`.

use std::fs;
use std::io::Write;
use std::path::Path;

use crate::targets::TARGETS;

use super::shims::{is_ogt_shim, render_shim};

/// Install — or idempotently regenerate — one shim per
/// `crate::targets::TARGETS` entry into `dir`. Refuses with a non-zero
/// exit if `dir` exists and is not a directory; creates it if missing.
/// Every run overwrites ogt's own files unconditionally and never
/// touches anything else already in `dir` — re-running this is exactly
/// regeneration, which is required after ogt is upgraded with a
/// different target list.
pub(crate) fn install_shims(
    dir: &Path,
    ogt_exe: &Path,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    if dir.exists() && !dir.is_dir() {
        let _ = writeln!(
            err,
            "ogt init --shims: {} exists and is not a directory",
            dir.display()
        );
        return 1;
    }
    if let Err(e) = fs::create_dir_all(dir) {
        let _ = writeln!(
            err,
            "ogt init --shims: could not create {}: {e}",
            dir.display()
        );
        return 1;
    }

    let ogt_exe_str = ogt_exe.to_string_lossy().into_owned();
    for program in TARGETS {
        let script = render_shim(&ogt_exe_str, program);
        let path = dir.join(program);
        if let Err(e) = write_executable(&path, &script) {
            let _ = writeln!(
                err,
                "ogt init --shims: could not write {}: {e}",
                path.display()
            );
            return 1;
        }
    }

    let _ = writeln!(
        out,
        "Installed {} shim(s) in {}",
        TARGETS.len(),
        dir.display()
    );
    let _ = writeln!(
        out,
        "\nAdd this to PATH — it must come FIRST, or the shims are inert:\n\n    export PATH=\"{}:$PATH\"\n",
        dir.display()
    );
    let _ = writeln!(
        out,
        "With shims on PATH, the Claude Code hook (`ogt init`, no `--shims`) is \
         redundant; both may still be installed at once."
    );
    0
}

#[cfg(unix)]
fn write_executable(path: &Path, contents: &str) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::write(path, contents)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))
}

#[cfg(not(unix))]
fn write_executable(path: &Path, contents: &str) -> std::io::Result<()> {
    fs::write(path, contents)
}

/// Remove exactly ogt's own shims from `dir` — every file whose contents
/// carry `shims`'s marker. A file without the marker is never touched,
/// even if its name matches one of `TARGETS`. A missing `dir` is a no-op.
pub(crate) fn uninstall_shims(dir: &Path, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    if !dir.exists() {
        let _ = writeln!(out, "{} does not exist; nothing to do", dir.display());
        return 0;
    }
    if !dir.is_dir() {
        let _ = writeln!(
            err,
            "ogt init --shims --uninstall: {} exists and is not a directory",
            dir.display()
        );
        return 1;
    }

    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            let _ = writeln!(
                err,
                "ogt init --shims --uninstall: could not read {}: {e}",
                dir.display()
            );
            return 1;
        }
    };

    let mut removed = 0usize;
    let mut skipped: Vec<String> = Vec::new();

    for entry in entries {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        match fs::read_to_string(&path) {
            Ok(contents) if is_ogt_shim(&contents) => match fs::remove_file(&path) {
                Ok(()) => removed += 1,
                Err(_) => skipped.push(name),
            },
            _ => skipped.push(name),
        }
    }

    let _ = writeln!(out, "Removed {removed} ogt shim(s) from {}", dir.display());
    if !skipped.is_empty() {
        let _ = writeln!(out, "Left untouched (not ogt's): {}", skipped.join(", "));
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn is_executable(path: &Path) -> bool {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path)
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }

    #[test]
    #[cfg(unix)]
    fn install_generates_one_marked_executable_shim_per_target() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("shims");
        let ogt_exe = tmp.path().join("ogt");
        let mut out = Vec::new();
        let mut err = Vec::new();

        let code = install_shims(&dir, &ogt_exe, &mut out, &mut err);
        assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&err));

        for program in TARGETS {
            let path = dir.join(program);
            assert!(path.exists(), "missing shim for {program}");
            assert!(is_executable(&path), "shim for {program} is not executable");
            let contents = fs::read_to_string(&path).unwrap();
            assert!(is_ogt_shim(&contents));
            assert!(contents.contains(&ogt_exe.to_string_lossy().into_owned()));
        }
        assert!(String::from_utf8_lossy(&out).contains("export PATH="));
    }

    #[test]
    fn install_refuses_when_dir_exists_and_is_not_a_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let not_a_dir = tmp.path().join("shims");
        fs::write(&not_a_dir, "not a directory").unwrap();
        let ogt_exe = tmp.path().join("ogt");
        let mut out = Vec::new();
        let mut err = Vec::new();

        let code = install_shims(&not_a_dir, &ogt_exe, &mut out, &mut err);
        assert_eq!(code, 1);
        assert!(out.is_empty());
        assert!(!err.is_empty());
        // Refused: the file on disk must be untouched.
        assert_eq!(fs::read_to_string(&not_a_dir).unwrap(), "not a directory");
    }

    #[test]
    #[cfg(unix)]
    fn install_is_idempotent_and_regenerates_on_rerun() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("shims");
        let ogt_exe = tmp.path().join("ogt");
        install_shims(&dir, &ogt_exe, &mut Vec::new(), &mut Vec::new());
        let first = fs::read_to_string(dir.join("grep")).unwrap();

        let code = install_shims(&dir, &ogt_exe, &mut Vec::new(), &mut Vec::new());
        assert_eq!(code, 0);
        let second = fs::read_to_string(dir.join("grep")).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    #[cfg(unix)]
    fn uninstall_removes_only_marked_files() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("shims");
        let ogt_exe = tmp.path().join("ogt");
        install_shims(&dir, &ogt_exe, &mut Vec::new(), &mut Vec::new());

        // A file ogt did not create must survive uninstall untouched.
        let unmarked = dir.join("my-own-script");
        fs::write(&unmarked, "#!/bin/sh\necho mine\n").unwrap();

        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = uninstall_shims(&dir, &mut out, &mut err);
        assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&err));

        for program in TARGETS {
            assert!(
                !dir.join(program).exists(),
                "{program} shim must be removed"
            );
        }
        assert!(unmarked.exists(), "unmarked file must survive uninstall");
        assert_eq!(
            fs::read_to_string(&unmarked).unwrap(),
            "#!/bin/sh\necho mine\n"
        );

        let printed = String::from_utf8_lossy(&out);
        assert!(printed.contains(&format!("Removed {} ogt shim(s)", TARGETS.len())));
        assert!(printed.contains("my-own-script"));
    }

    #[test]
    fn uninstall_on_missing_dir_is_a_no_op() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("does-not-exist");
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = uninstall_shims(&dir, &mut out, &mut err);
        assert_eq!(code, 0);
        assert!(err.is_empty());
    }

    #[test]
    fn uninstall_refuses_when_dir_exists_and_is_not_a_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let not_a_dir = tmp.path().join("shims");
        fs::write(&not_a_dir, "not a directory").unwrap();
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = uninstall_shims(&not_a_dir, &mut out, &mut err);
        assert_eq!(code, 1);
        assert_eq!(fs::read_to_string(&not_a_dir).unwrap(), "not a directory");
    }
}
