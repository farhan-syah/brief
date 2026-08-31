//! `ogt init`'s own argv parsing and entry point — installs or
//! uninstalls the PreToolUse hook in the user's Claude Code settings.json,
//! or (with `--shims <dir>`) generates/removes PATH shims. With no flags
//! at all and a terminal on stdin, runs the interactive flow in
//! `interactive` instead of installing immediately — see `run_with`'s
//! decision. Manual flag parsing, matching `report::cli`'s style (no
//! dependency).

use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

use super::fs_ops;
use super::interactive;
use super::shim_fs;

const USAGE: &str = "usage: ogt init [--dry-run] [--uninstall] [--shims <dir>] [--yes]\n";

/// Text for `ogt init --help`. A function, not a `const`, because the
/// scope line names every program in `crate::targets::TARGETS`.
fn help_text() -> String {
    format!(
        "\
ogt init — install ogt's PreToolUse hook in Claude Code's settings.json,
or generate PATH shims that work with any harness and any agent.

The hook rewrites plain {targets} Bash calls to go through ogt,
so their output is gated behind the same token threshold as a direct
`ogt grep ...` invocation. It never changes what a command means: any
command it cannot confidently classify is left alone untouched.

Idempotent: running this again when the hook is already installed does
nothing. Every write backs up the previous settings.json to
settings.json.bak first, and is written atomically.

Run with no flags on a terminal: walks through mechanism, scope, and a
preview before writing anything, and prints the exact undo command
afterward. `--yes` skips straight to today's immediate, non-interactive
install; a script or CI pipe (stdin not a terminal) always gets the
immediate install too, `--yes` or not.

Flags:
  --dry-run       print what would change; touch nothing (hook install only)
  --uninstall     remove exactly ogt's own hook entry, or with --shims
                  remove exactly ogt's own shims; absent is a no-op
  --shims <dir>   generate one PATH shim per {targets} program into <dir>
  --yes           skip the interactive flow; install immediately
  --help, -h      this text

PATH shims are a directory of small wrapper scripts placed early on PATH.
Unlike the hook above, they work with every shell and every agent,
including one that spawns commands with no shell at all, and need no
per-harness adapter. `ogt init --shims <dir>` (re-)generates one shim
per program in ogt's target list and prints the exact PATH line to add;
the shim directory must come FIRST on PATH or the shims are inert.
Re-run the same command to regenerate the shims after ogt is upgraded
with a different target list. `--shims <dir> --uninstall` removes only
files carrying ogt's own marker, leaving anything else already in <dir>
untouched. With shims on PATH, the hook above is redundant; both may be
installed at once, that is not an error, only your choice.

Usage: ogt init [--dry-run] [--uninstall] [--yes]
       ogt init --shims <dir> [--uninstall]

To run a program literally named \"init\", invoke it by path:
ogt ./init
",
        targets = crate::targets::slash_list()
    )
}

struct ParsedArgs<'a> {
    dry_run: bool,
    uninstall: bool,
    yes: bool,
    shims_dir: Option<&'a str>,
}

enum ParseOutcome<'a> {
    Help,
    Usage,
    Args(ParsedArgs<'a>),
}

fn parse_args(args: &[String]) -> ParseOutcome<'_> {
    let mut dry_run = false;
    let mut uninstall = false;
    let mut yes = false;
    let mut shims_dir: Option<&str> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--help" | "-h" => return ParseOutcome::Help,
            "--dry-run" => dry_run = true,
            "--uninstall" => uninstall = true,
            "--yes" => yes = true,
            "--shims" => {
                let Some(value) = args.get(i + 1) else {
                    return ParseOutcome::Usage;
                };
                shims_dir = Some(value.as_str());
                i += 1;
            }
            _ => return ParseOutcome::Usage,
        }
        i += 1;
    }

    ParseOutcome::Args(ParsedArgs {
        dry_run,
        uninstall,
        yes,
        shims_dir,
    })
}

/// Entry point wired from `cli::dispatch` for `ogt init [...]`.
/// `args` is the argv following the literal `init` token.
pub(crate) fn run(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let stdin = std::io::stdin();
    let stdin_is_terminal = stdin.is_terminal();
    let mut reader = stdin.lock();
    run_with(
        args,
        out,
        err,
        dirs::home_dir().as_deref(),
        std::env::current_exe().ok().as_deref(),
        dirs::data_local_dir().map(|d| d.join("ogt").join("shims")),
        dirs::config_dir(),
        stdin_is_terminal,
        &mut reader,
    )
}

/// `run` with the home/shim/config directories, ogt's own executable
/// path, terminal-ness, and the stdin reader all injected, so tests can
/// drive this as a pure function against a tempdir and a scripted reader
/// instead of the real `~/.claude`, real data/config dirs, the real
/// running binary, and a real terminal.
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_with(
    args: &[String],
    out: &mut dyn Write,
    err: &mut dyn Write,
    home_dir: Option<&Path>,
    ogt_exe: Option<&Path>,
    default_shims_dir: Option<PathBuf>,
    config_dir: Option<PathBuf>,
    stdin_is_terminal: bool,
    reader: &mut dyn BufRead,
) -> i32 {
    let parsed = match parse_args(args) {
        ParseOutcome::Help => {
            let _ = out.write_all(help_text().as_bytes());
            return 0;
        }
        ParseOutcome::Usage => {
            let _ = err.write_all(USAGE.as_bytes());
            return 2;
        }
        ParseOutcome::Args(p) => p,
    };

    if let Some(dir) = parsed.shims_dir {
        return run_shims(Path::new(dir), parsed.uninstall, ogt_exe, out, err);
    }

    // Interactive only for the plain install path: no --dry-run,
    // --uninstall, or --yes, and stdin is a real terminal. A script or CI
    // pipe (stdin not a terminal) always gets today's immediate install,
    // unconditionally — this is a hard requirement, not a default that
    // --yes merely happens to also satisfy.
    if !parsed.dry_run && !parsed.uninstall && !parsed.yes && stdin_is_terminal {
        return interactive::run(
            out,
            reader,
            home_dir,
            default_shims_dir,
            config_dir,
            ogt_exe,
        );
    }

    fs_ops::run_with(home_dir, parsed.dry_run, parsed.uninstall, out, err)
}

/// The `--shims <dir>` branch: install (regenerate) or uninstall, wired to
/// `shim_fs`. `ogt_exe` is `None` only when `std::env::current_exe()`
/// itself failed — installing then has nothing correct to reference, so
/// it is refused rather than falling back to a bare `ogt` that a shim's
/// own directory could shadow.
fn run_shims(
    dir: &Path,
    uninstall: bool,
    ogt_exe: Option<&Path>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    if uninstall {
        return shim_fs::uninstall_shims(dir, out, err);
    }
    let Some(ogt_exe) = ogt_exe else {
        let _ = writeln!(
            err,
            "ogt init --shims: could not determine ogt's own executable path"
        );
        return 1;
    };
    shim_fs::install_shims(dir, ogt_exe, out, err)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every existing test drives the non-interactive path: `stdin_is_terminal`
    /// is always `false` here, and the reader is empty since it must never be
    /// consulted on that path.
    fn run_noninteractive(
        args: &[String],
        out: &mut dyn Write,
        err: &mut dyn Write,
        home_dir: Option<&Path>,
        ogt_exe: Option<&Path>,
    ) -> i32 {
        let mut empty: &[u8] = &[];
        run_with(
            args, out, err, home_dir, ogt_exe, None, None, false, &mut empty,
        )
    }

    #[test]
    fn help_flag_prints_help_and_exits_0() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run_noninteractive(&["--help".to_string()], &mut out, &mut err, None, None);
        assert_eq!(code, 0);
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("Idempotent:"));
        assert!(err.is_empty());
    }

    #[test]
    fn help_flag_documents_shims() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        run_noninteractive(&["--help".to_string()], &mut out, &mut err, None, None);
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("--shims <dir>"));
        assert!(text.contains("must come FIRST"));
        assert!(text.contains("regenerate"));
        assert!(text.contains("redundant"));
    }

    #[test]
    fn help_flag_documents_yes_and_interactive() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        run_noninteractive(&["--help".to_string()], &mut out, &mut err, None, None);
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("--yes"));
        assert!(text.contains("terminal"));
    }

    #[test]
    fn short_help_flag_also_works() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run_noninteractive(&["-h".to_string()], &mut out, &mut err, None, None);
        assert_eq!(code, 0);
        assert!(!out.is_empty());
    }

    #[test]
    fn unknown_flag_is_a_usage_error() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run_noninteractive(&["--bogus".to_string()], &mut out, &mut err, None, None);
        assert_eq!(code, 2);
        assert!(out.is_empty());
        assert!(!err.is_empty());
    }

    #[test]
    fn shims_flag_missing_value_is_a_usage_error() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run_noninteractive(&["--shims".to_string()], &mut out, &mut err, None, None);
        assert_eq!(code, 2);
        assert!(out.is_empty());
        assert!(!err.is_empty());
    }

    #[test]
    fn no_home_dir_injected_is_an_error_not_a_panic() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run_noninteractive(&[], &mut out, &mut err, None, None);
        assert_eq!(code, 1);
    }

    #[test]
    fn dry_run_and_uninstall_flags_parse_and_reach_fs_ops() {
        let tmp = tempfile::tempdir().unwrap();
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run_noninteractive(
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
    fn yes_flag_takes_the_immediate_install_path_even_conceptually_on_a_terminal() {
        // stdin_is_terminal is forced true here specifically to prove --yes
        // overrides it — every other test in this module forces it false.
        let tmp = tempfile::tempdir().unwrap();
        let mut out = Vec::new();
        let mut err = Vec::new();
        let mut empty: &[u8] = &[];
        let code = run_with(
            &["--yes".to_string()],
            &mut out,
            &mut err,
            Some(tmp.path()),
            None,
            None,
            None,
            true,
            &mut empty,
        );
        assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&err));
        assert!(tmp.path().join(".claude").join("settings.json").exists());
    }

    #[test]
    fn non_terminal_stdin_takes_the_immediate_install_path() {
        let tmp = tempfile::tempdir().unwrap();
        let mut out = Vec::new();
        let mut err = Vec::new();
        let mut empty: &[u8] = &[];
        let code = run_with(
            &[],
            &mut out,
            &mut err,
            Some(tmp.path()),
            None,
            None,
            None,
            false,
            &mut empty,
        );
        assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&err));
        assert!(tmp.path().join(".claude").join("settings.json").exists());
    }

    #[test]
    fn terminal_stdin_with_no_flags_goes_interactive_and_writes_nothing_on_decline() {
        let tmp = tempfile::tempdir().unwrap();
        let mut out = Vec::new();
        let mut err = Vec::new();
        let mut script: &[u8] = b"1\n1\nn\n"; // hook, everywhere, decline
        let code = run_with(
            &[],
            &mut out,
            &mut err,
            Some(tmp.path()),
            None,
            None,
            None,
            true,
            &mut script,
        );
        assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&err));
        assert!(!tmp.path().join(".claude").join("settings.json").exists());
        assert!(String::from_utf8_lossy(&out).contains("Nothing installed"));
    }

    #[test]
    fn shims_flag_installs_when_ogt_exe_is_injected() {
        let tmp = tempfile::tempdir().unwrap();
        let shims_dir = tmp.path().join("shims");
        let ogt_exe = tmp.path().join("ogt");
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run_noninteractive(
            &[
                "--shims".to_string(),
                shims_dir.to_string_lossy().into_owned(),
            ],
            &mut out,
            &mut err,
            None,
            Some(&ogt_exe),
        );
        assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&err));
        for program in crate::targets::TARGETS {
            assert!(shims_dir.join(program).exists());
        }
        // The hook path (home_dir) must never be touched by --shims.
        assert!(!tmp.path().join(".claude").exists());
    }

    #[test]
    fn shims_flag_without_injected_ogt_exe_refuses_rather_than_guessing() {
        let tmp = tempfile::tempdir().unwrap();
        let shims_dir = tmp.path().join("shims");
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run_noninteractive(
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
    fn shims_flag_with_uninstall_removes_shims_without_needing_ogt_exe() {
        let tmp = tempfile::tempdir().unwrap();
        let shims_dir = tmp.path().join("shims");
        let ogt_exe = tmp.path().join("ogt");
        run_noninteractive(
            &[
                "--shims".to_string(),
                shims_dir.to_string_lossy().into_owned(),
            ],
            &mut Vec::new(),
            &mut Vec::new(),
            None,
            Some(&ogt_exe),
        );

        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run_noninteractive(
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
