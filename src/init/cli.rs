//! `brief init`'s own argv parsing and entry point — installs or
//! uninstalls the PreToolUse hook in the user's Claude Code settings.json,
//! or (with `--shims <dir>`) generates/removes PATH shims. Manual flag
//! parsing, matching `report::cli`'s style (no dependency).

use std::io::Write;
use std::path::Path;

use super::fs_ops;
use super::shim_fs;

const USAGE: &str = "usage: brief init [--dry-run] [--uninstall] [--shims <dir>]\n";

/// Text for `brief init --help`. A function, not a `const`, because the
/// scope line names every program in `crate::targets::TARGETS`.
fn help_text() -> String {
    format!(
        "\
brief init — install brief's PreToolUse hook in Claude Code's settings.json,
or generate PATH shims that work with any harness and any agent.

The hook rewrites plain {targets} Bash calls to go through brief,
so their output is gated behind the same token threshold as a direct
`brief grep ...` invocation. It never changes what a command means: any
command it cannot confidently classify is left alone untouched.

Idempotent: running this again when the hook is already installed does
nothing. Every write backs up the previous settings.json to
settings.json.bak first, and is written atomically.

Flags:
  --dry-run       print what would change; touch nothing (hook install only)
  --uninstall     remove exactly brief's own hook entry, or with --shims
                  remove exactly brief's own shims; absent is a no-op
  --shims <dir>   generate one PATH shim per {targets} program into <dir>
  --help, -h      this text

PATH shims are a directory of small wrapper scripts placed early on PATH.
Unlike the hook above, they work with every shell and every agent,
including one that spawns commands with no shell at all, and need no
per-harness adapter. `brief init --shims <dir>` (re-)generates one shim
per program in brief's target list and prints the exact PATH line to add;
the shim directory must come FIRST on PATH or the shims are inert.
Re-run the same command to regenerate the shims after brief is upgraded
with a different target list. `--shims <dir> --uninstall` removes only
files carrying brief's own marker, leaving anything else already in <dir>
untouched. With shims on PATH, the hook above is redundant; both may be
installed at once, that is not an error, only your choice.

Usage: brief init [--dry-run] [--uninstall]
       brief init --shims <dir> [--uninstall]

To run a program literally named \"init\", invoke it by path:
brief ./init
",
        targets = crate::targets::slash_list()
    )
}

/// Entry point wired from `cli::dispatch` for `brief init [...]`.
/// `args` is the argv following the literal `init` token.
pub(crate) fn run(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    run_with(
        args,
        out,
        err,
        dirs::home_dir().as_deref(),
        std::env::current_exe().ok().as_deref(),
    )
}

/// `run` with the home directory and brief's own executable path injected,
/// so tests can drive this as a pure function against a tempdir instead of
/// the real `~/.claude` and the real running binary.
pub(crate) fn run_with(
    args: &[String],
    out: &mut dyn Write,
    err: &mut dyn Write,
    home_dir: Option<&Path>,
    brief_exe: Option<&Path>,
) -> i32 {
    let mut dry_run = false;
    let mut uninstall = false;
    let mut shims_dir: Option<&str> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--help" | "-h" => {
                let _ = out.write_all(help_text().as_bytes());
                return 0;
            }
            "--dry-run" => dry_run = true,
            "--uninstall" => uninstall = true,
            "--shims" => {
                let Some(value) = args.get(i + 1) else {
                    let _ = err.write_all(USAGE.as_bytes());
                    return 2;
                };
                shims_dir = Some(value.as_str());
                i += 1;
            }
            _ => {
                let _ = err.write_all(USAGE.as_bytes());
                return 2;
            }
        }
        i += 1;
    }

    if let Some(dir) = shims_dir {
        return run_shims(Path::new(dir), uninstall, brief_exe, out, err);
    }

    fs_ops::run_with(home_dir, dry_run, uninstall, out, err)
}

/// The `--shims <dir>` branch: install (regenerate) or uninstall, wired to
/// `shim_fs`. `brief_exe` is `None` only when `std::env::current_exe()`
/// itself failed — installing then has nothing correct to reference, so
/// it is refused rather than falling back to a bare `brief` that a shim's
/// own directory could shadow.
fn run_shims(
    dir: &Path,
    uninstall: bool,
    brief_exe: Option<&Path>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    if uninstall {
        return shim_fs::uninstall_shims(dir, out, err);
    }
    let Some(brief_exe) = brief_exe else {
        let _ = writeln!(
            err,
            "brief init --shims: could not determine brief's own executable path"
        );
        return 1;
    };
    shim_fs::install_shims(dir, brief_exe, out, err)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_flag_prints_help_and_exits_0() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run_with(&["--help".to_string()], &mut out, &mut err, None, None);
        assert_eq!(code, 0);
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("Idempotent:"));
        assert!(err.is_empty());
    }

    #[test]
    fn help_flag_documents_shims() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        run_with(&["--help".to_string()], &mut out, &mut err, None, None);
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("--shims <dir>"));
        assert!(text.contains("must come FIRST"));
        assert!(text.contains("regenerate"));
        assert!(text.contains("redundant"));
    }

    #[test]
    fn short_help_flag_also_works() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run_with(&["-h".to_string()], &mut out, &mut err, None, None);
        assert_eq!(code, 0);
        assert!(!out.is_empty());
    }

    #[test]
    fn unknown_flag_is_a_usage_error() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run_with(&["--bogus".to_string()], &mut out, &mut err, None, None);
        assert_eq!(code, 2);
        assert!(out.is_empty());
        assert!(!err.is_empty());
    }

    #[test]
    fn shims_flag_missing_value_is_a_usage_error() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run_with(&["--shims".to_string()], &mut out, &mut err, None, None);
        assert_eq!(code, 2);
        assert!(out.is_empty());
        assert!(!err.is_empty());
    }

    #[test]
    fn no_home_dir_injected_is_an_error_not_a_panic() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run_with(&[], &mut out, &mut err, None, None);
        assert_eq!(code, 1);
    }

    #[test]
    fn dry_run_and_uninstall_flags_parse_and_reach_fs_ops() {
        let tmp = tempfile::tempdir().unwrap();
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run_with(
            &["--dry-run".to_string(), "--uninstall".to_string()],
            &mut out,
            &mut err,
            Some(tmp.path()),
            None,
        );
        assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&err));
        assert!(!tmp.path().join(".claude").join("settings.json").exists());
    }

    #[test]
    fn shims_flag_installs_when_brief_exe_is_injected() {
        let tmp = tempfile::tempdir().unwrap();
        let shims_dir = tmp.path().join("shims");
        let brief_exe = tmp.path().join("brief");
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run_with(
            &[
                "--shims".to_string(),
                shims_dir.to_string_lossy().into_owned(),
            ],
            &mut out,
            &mut err,
            None,
            Some(&brief_exe),
        );
        assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&err));
        for program in crate::targets::TARGETS {
            assert!(shims_dir.join(program).exists());
        }
        // The hook path (home_dir) must never be touched by --shims.
        assert!(!tmp.path().join(".claude").exists());
    }

    #[test]
    fn shims_flag_without_injected_brief_exe_refuses_rather_than_guessing() {
        let tmp = tempfile::tempdir().unwrap();
        let shims_dir = tmp.path().join("shims");
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run_with(
            &[
                "--shims".to_string(),
                shims_dir.to_string_lossy().into_owned(),
            ],
            &mut out,
            &mut err,
            None,
            None,
        );
        assert_eq!(code, 1);
        assert!(!shims_dir.exists());
    }

    #[test]
    fn shims_flag_with_uninstall_removes_shims_without_needing_brief_exe() {
        let tmp = tempfile::tempdir().unwrap();
        let shims_dir = tmp.path().join("shims");
        let brief_exe = tmp.path().join("brief");
        run_with(
            &[
                "--shims".to_string(),
                shims_dir.to_string_lossy().into_owned(),
            ],
            &mut Vec::new(),
            &mut Vec::new(),
            None,
            Some(&brief_exe),
        );

        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run_with(
            &[
                "--shims".to_string(),
                shims_dir.to_string_lossy().into_owned(),
                "--uninstall".to_string(),
            ],
            &mut out,
            &mut err,
            None,
            None,
        );
        assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&err));
        for program in crate::targets::TARGETS {
            assert!(!shims_dir.join(program).exists());
        }
    }
}
