//! `brief init`'s own argv parsing and entry point — installs or
//! uninstalls the PreToolUse hook in the user's Claude Code settings.json.
//! Manual flag parsing, matching `report::cli`'s style (no dependency).

use std::io::Write;

use super::fs_ops;

const USAGE: &str = "usage: brief init [--dry-run] [--uninstall]\n";

/// Text for `brief init --help`. A function, not a `const`, because the
/// scope line names every program in `crate::targets::TARGETS`.
fn help_text() -> String {
    format!(
        "\
brief init — install brief's PreToolUse hook in Claude Code's settings.json.

The hook rewrites plain {} Bash calls to go through brief,
so their output is gated behind the same token threshold as a direct
`brief grep ...` invocation. It never changes what a command means: any
command it cannot confidently classify is left alone untouched.

Idempotent: running this again when the hook is already installed does
nothing. Every write backs up the previous settings.json to
settings.json.bak first, and is written atomically.

Flags:
  --dry-run     print what would change; touch nothing
  --uninstall   remove exactly brief's own hook entry; absent is a no-op
  --help, -h    this text

Usage: brief init [--dry-run] [--uninstall]

To run a program literally named \"init\", invoke it by path:
brief ./init
",
        crate::targets::slash_list()
    )
}

/// Entry point wired from `cli::dispatch` for `brief init [...]`.
/// `args` is the argv following the literal `init` token.
pub(crate) fn run(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    run_with(args, out, err, dirs::home_dir().as_deref())
}

/// `run` with the home directory injected, so tests can drive this as a
/// pure function against a tempdir instead of the real `~/.claude`.
pub(crate) fn run_with(
    args: &[String],
    out: &mut dyn Write,
    err: &mut dyn Write,
    home_dir: Option<&std::path::Path>,
) -> i32 {
    let mut dry_run = false;
    let mut uninstall = false;

    for arg in args {
        match arg.as_str() {
            "--help" | "-h" => {
                let _ = out.write_all(help_text().as_bytes());
                return 0;
            }
            "--dry-run" => dry_run = true,
            "--uninstall" => uninstall = true,
            _ => {
                let _ = err.write_all(USAGE.as_bytes());
                return 2;
            }
        }
    }

    fs_ops::run_with(home_dir, dry_run, uninstall, out, err)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_flag_prints_help_and_exits_0() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run_with(&["--help".to_string()], &mut out, &mut err, None);
        assert_eq!(code, 0);
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("Idempotent:"));
        assert!(err.is_empty());
    }

    #[test]
    fn short_help_flag_also_works() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run_with(&["-h".to_string()], &mut out, &mut err, None);
        assert_eq!(code, 0);
        assert!(!out.is_empty());
    }

    #[test]
    fn unknown_flag_is_a_usage_error() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run_with(&["--bogus".to_string()], &mut out, &mut err, None);
        assert_eq!(code, 2);
        assert!(out.is_empty());
        assert!(!err.is_empty());
    }

    #[test]
    fn no_home_dir_injected_is_an_error_not_a_panic() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run_with(&[], &mut out, &mut err, None);
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
        );
        assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&err));
        assert!(!tmp.path().join(".claude").join("settings.json").exists());
    }
}
