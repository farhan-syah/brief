//! Filesystem side of `brief init`: locating settings.json under an
//! injected home directory, backup-before-write, atomic write, and
//! `--dry-run` gating. No JSON logic here — that's `settings_edit`, kept
//! pure and filesystem-free so it stays unit-testable against literal
//! fixtures.

use std::fs;
use std::io::Write;
use std::path::Path;

use super::settings_edit::{InsertOutcome, RemoveOutcome, hook_entry_template};
use super::settings_edit::{insert_hook_entry, remove_hook_entry};

/// Run `brief init`. `home_dir` is injected so tests can point this at a
/// tempdir instead of the real `~/.claude` — see the module doc comment
/// on `crate::init` for why that boundary is not optional here.
pub(crate) fn run_with(
    home_dir: Option<&Path>,
    dry_run: bool,
    uninstall: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let Some(home_dir) = home_dir else {
        let _ = writeln!(err, "brief init: could not determine the home directory");
        return 1;
    };
    let settings_path = home_dir.join(".claude").join("settings.json");

    let (contents, existed) = match fs::read_to_string(&settings_path) {
        Ok(s) => (s, true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => ("{}".to_string(), false),
        Err(e) => {
            let _ = writeln!(
                err,
                "brief init: could not read {}: {e}",
                settings_path.display()
            );
            return 1;
        }
    };

    if uninstall {
        run_uninstall(&settings_path, &contents, existed, dry_run, out, err)
    } else {
        run_install(&settings_path, &contents, existed, dry_run, out, err)
    }
}

fn run_install(
    settings_path: &Path,
    contents: &str,
    existed: bool,
    dry_run: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    match insert_hook_entry(contents) {
        Ok(InsertOutcome::AlreadyPresent) => {
            let _ = writeln!(
                out,
                "brief hook already installed in {}; nothing to do",
                settings_path.display()
            );
            0
        }
        Ok(InsertOutcome::Inserted(new_contents)) => {
            if dry_run {
                let _ = writeln!(
                    out,
                    "Would install the brief PreToolUse hook in {}:\n",
                    settings_path.display()
                );
                let _ = writeln!(out, "{new_contents}");
                return 0;
            }
            match write_settings(settings_path, contents, &new_contents, existed) {
                Ok(()) => {
                    let _ = writeln!(
                        out,
                        "Installed the brief PreToolUse hook in {}",
                        settings_path.display()
                    );
                    0
                }
                Err(e) => {
                    let _ = writeln!(err, "brief init: {e}");
                    1
                }
            }
        }
        Err(_unrecognized) => {
            let _ = writeln!(
                err,
                "brief init: {} isn't a shape I confidently recognize, so I won't guess at editing it.\n\
                 Add this entry to your `hooks.PreToolUse` array by hand:\n",
                settings_path.display()
            );
            let _ = writeln!(err, "{}", hook_entry_template());
            1
        }
    }
}

fn run_uninstall(
    settings_path: &Path,
    contents: &str,
    existed: bool,
    dry_run: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    match remove_hook_entry(contents) {
        Ok(RemoveOutcome::NotPresent) => {
            let _ = writeln!(
                out,
                "brief hook not installed in {}; nothing to do",
                settings_path.display()
            );
            0
        }
        Ok(RemoveOutcome::Removed(new_contents)) => {
            if dry_run {
                let _ = writeln!(
                    out,
                    "Would remove the brief PreToolUse hook from {}:\n",
                    settings_path.display()
                );
                let _ = writeln!(out, "{new_contents}");
                return 0;
            }
            match write_settings(settings_path, contents, &new_contents, existed) {
                Ok(()) => {
                    let _ = writeln!(
                        out,
                        "Removed the brief PreToolUse hook from {}",
                        settings_path.display()
                    );
                    0
                }
                Err(e) => {
                    let _ = writeln!(err, "brief init: {e}");
                    1
                }
            }
        }
        Err(_unrecognized) => {
            let _ = writeln!(
                err,
                "brief init --uninstall: {} isn't a shape I confidently recognize, \
                 so I won't guess at editing it. Remove the brief hook entry from \
                 `hooks.PreToolUse` by hand.",
                settings_path.display()
            );
            1
        }
    }
}

/// Back up the current file (only if it already existed — there is
/// nothing real to protect by "backing up" a file that never existed),
/// then write `new_contents` atomically: a temp file in the same
/// directory, then a rename, so a reader never observes a truncated file.
fn write_settings(
    settings_path: &Path,
    old_contents: &str,
    new_contents: &str,
    existed: bool,
) -> std::io::Result<()> {
    let dir = settings_path.parent().ok_or_else(|| {
        std::io::Error::other(format!(
            "{} has no parent directory",
            settings_path.display()
        ))
    })?;
    fs::create_dir_all(dir)?;

    if existed {
        let backup_path = settings_path.with_extension("json.bak");
        fs::write(&backup_path, old_contents)?;
    }

    let tmp_path = dir.join(format!(".brief-settings-{}.tmp", std::process::id()));
    if let Err(e) = fs::write(&tmp_path, new_contents) {
        let _ = fs::remove_file(&tmp_path);
        return Err(e);
    }
    if let Err(e) = fs::rename(&tmp_path, settings_path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(e);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::settings_edit::find_hook_entry;
    use super::*;
    use std::path::PathBuf;

    fn home(tmp: &Path) -> PathBuf {
        tmp.to_path_buf()
    }

    fn settings_path(tmp: &Path) -> PathBuf {
        tmp.join(".claude").join("settings.json")
    }

    #[test]
    fn no_home_dir_is_an_error() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run_with(None, false, false, &mut out, &mut err);
        assert_eq!(code, 1);
        assert!(out.is_empty());
        assert!(!err.is_empty());
    }

    #[test]
    fn install_creates_settings_json_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run_with(Some(&home(tmp.path())), false, false, &mut out, &mut err);
        assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&err));

        let written = fs::read_to_string(settings_path(tmp.path())).unwrap();
        assert!(written.contains("brief hook"));
        assert!(find_hook_entry(&written).unwrap());

        // No pre-existing file: nothing to back up.
        assert!(
            !settings_path(tmp.path())
                .with_extension("json.bak")
                .exists()
        );
    }

    #[test]
    fn install_is_idempotent_on_second_run() {
        let tmp = tempfile::tempdir().unwrap();
        let mut out = Vec::new();
        let mut err = Vec::new();
        run_with(Some(&home(tmp.path())), false, false, &mut out, &mut err);
        let first = fs::read_to_string(settings_path(tmp.path())).unwrap();

        let mut out2 = Vec::new();
        let mut err2 = Vec::new();
        let code = run_with(Some(&home(tmp.path())), false, false, &mut out2, &mut err2);
        assert_eq!(code, 0);
        let second = fs::read_to_string(settings_path(tmp.path())).unwrap();
        assert_eq!(first, second, "a second install must not change the file");
        assert!(String::from_utf8_lossy(&out2).contains("already installed"));
    }

    #[test]
    fn install_backs_up_an_existing_file_before_writing() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join(".claude")).unwrap();
        let original = r#"{"env":{"FOO":"bar"}}"#;
        fs::write(settings_path(tmp.path()), original).unwrap();

        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run_with(Some(&home(tmp.path())), false, false, &mut out, &mut err);
        assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&err));

        let backup = fs::read_to_string(settings_path(tmp.path()).with_extension("json.bak"))
            .expect("backup file must exist");
        assert_eq!(backup, original);

        let written = fs::read_to_string(settings_path(tmp.path())).unwrap();
        assert!(written.contains(r#""FOO":"bar""#));
        assert!(find_hook_entry(&written).unwrap());
    }

    #[test]
    fn dry_run_touches_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run_with(Some(&home(tmp.path())), true, false, &mut out, &mut err);
        assert_eq!(code, 0);
        assert!(!settings_path(tmp.path()).exists());
        assert!(!String::from_utf8_lossy(&out).is_empty());
    }

    #[test]
    fn uninstall_removes_the_entry() {
        let tmp = tempfile::tempdir().unwrap();
        run_with(
            Some(&home(tmp.path())),
            false,
            false,
            &mut Vec::new(),
            &mut Vec::new(),
        );

        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run_with(Some(&home(tmp.path())), false, true, &mut out, &mut err);
        assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&err));

        let written = fs::read_to_string(settings_path(tmp.path())).unwrap();
        assert!(!written.contains("brief hook"));
    }

    #[test]
    fn uninstall_when_absent_is_a_no_op_exit_0() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join(".claude")).unwrap();
        fs::write(settings_path(tmp.path()), "{}").unwrap();

        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run_with(Some(&home(tmp.path())), false, true, &mut out, &mut err);
        assert_eq!(code, 0);
        assert!(String::from_utf8_lossy(&out).contains("nothing to do"));
    }

    #[test]
    fn unrecognized_shape_refuses_to_write_and_prints_the_block() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join(".claude")).unwrap();
        fs::write(settings_path(tmp.path()), r#"{"hooks":"not an object"}"#).unwrap();

        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run_with(Some(&home(tmp.path())), false, false, &mut out, &mut err);
        assert_eq!(code, 1);
        let printed = String::from_utf8_lossy(&err);
        assert!(printed.contains("\"matcher\": \"Bash\""));
        assert!(printed.contains("brief hook"));

        // Refused: the file on disk must be exactly what it was before.
        let untouched = fs::read_to_string(settings_path(tmp.path())).unwrap();
        assert_eq!(untouched, r#"{"hooks":"not an object"}"#);
        assert!(
            !settings_path(tmp.path())
                .with_extension("json.bak")
                .exists()
        );
    }
}
