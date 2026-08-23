//! Argv dispatch for `sigfold <program> [args...]` — the `env`/`time`/`nice`
//! pattern. `argv[1]` names the child literally; everything after it is
//! forwarded byte-for-byte and never inspected or re-parsed. There is no
//! `--` separator and no argument-parsing dependency: a parser reserves
//! top-level `-h`/`--help`/`-V`/`--version`, and reserving them would make
//! `sigfold grep -h` silently print sigfold's help instead of grep's — a
//! correctness bug of exactly the kind this tool exists to avoid.
//!
//! The literal `argv[1]` value `report` is reserved the same way, for
//! `sigfold report [...]` (see `crate::report`). The trade-off is the same
//! one the flag reservations already accept: a program literally named
//! `report` can no longer be run as `sigfold report`, only by path
//! (`sigfold ./report`).

use std::ffi::OsString;
use std::io::Write;
use std::process::Command;

use crate::fold::FoldConfig;
use crate::report;
use crate::runner::{basename, run_with};
use crate::track::TrackConfig;

use super::help::{help_text, version};
use super::passthrough;

/// Programs sigfold folds, matched on the basename of `argv[1]`. Everything
/// else takes the passthrough path with fully inherited stdio. `help::help_text`
/// reuses this so the target names exist in exactly one place.
pub(super) const TARGETS: [&str; 4] = ["grep", "cat", "find", "rg"];

/// Printed on stderr when sigfold is invoked with no program at all.
const USAGE: &str = "usage: sigfold <program> [args...]\n";

/// Run the CLI and return the process exit code. Never panics.
///
/// `args` is the full process argv including `argv[0]`, exactly as
/// `std::env::args_os()` yields it — this lets tests drive `main_with`
/// identically to the real binary. `out`/`err` receive sigfold's own
/// output (help/usage/version) and the fold-gated stdout/stderr of a
/// target command; a non-target command's streams bypass both and go
/// straight to this process's real, inherited stdio.
pub fn main_with(
    args: impl Iterator<Item = OsString>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let post_program: Vec<OsString> = args.skip(1).collect();

    let Some(program) = post_program.first().cloned() else {
        let _ = err.write_all(USAGE.as_bytes());
        return 2;
    };

    // Intercept sigfold's own help/version ONLY when the entire
    // post-program argv is this one token — `sigfold grep --help` (len 2)
    // must reach grep untouched; only `sigfold --help` (len 1) is sigfold's.
    if post_program.len() == 1
        && let Some(flag) = program.to_str()
        && matches!(flag, "--help" | "-h" | "--version" | "-V")
    {
        let text = if matches!(flag, "--help" | "-h") {
            help_text()
        } else {
            version()
        };
        let _ = out.write_all(text.as_bytes());
        return 0;
    }

    // Reserved literal, intercepted before the fold-target branch below —
    // see the module doc comment for the trade-off this accepts.
    if program.to_str() == Some("report") {
        let report_args: Vec<String> = post_program[1..]
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        return report::run(&report_args, out, err);
    }

    let forwarded = &post_program[1..];
    let mut cmd = Command::new(&program);
    cmd.args(forwarded);

    if TARGETS.contains(&basename(&program).as_str()) {
        dispatch_target(
            cmd,
            &program,
            &FoldConfig::from_env(),
            &TrackConfig::from_env(),
            out,
            err,
        )
    } else {
        passthrough::run_passthrough(cmd, &program, err)
    }
}

/// Run a fold-target command under `cfg`/`track_cfg`. Split out from
/// `main_with` so tests can drive the fold-gate and tracking wiring with
/// explicit configs instead of the real process environment.
fn dispatch_target(
    cmd: Command,
    program: &OsString,
    cfg: &FoldConfig,
    track_cfg: &TrackConfig,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    match run_with(cmd, cfg, track_cfg, out, err) {
        Ok(outcome) => outcome.exit_code,
        Err(io_err) => passthrough::exit_code_for_spawn_error(&io_err, err, program),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(tokens: &[&str]) -> Vec<OsString> {
        tokens.iter().map(OsString::from).collect()
    }

    #[test]
    fn no_program_exits_2_and_spawns_nothing() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = main_with(argv(&["sigfold"]).into_iter(), &mut out, &mut err);
        assert_eq!(code, 2);
        assert!(out.is_empty());
        assert_eq!(err, USAGE.as_bytes());
    }

    #[test]
    fn bare_help_and_version_flags_print_sigfolds_own_text() {
        for flag in ["--help", "-h"] {
            let mut out = Vec::new();
            let mut err = Vec::new();
            let code = main_with(argv(&["sigfold", flag]).into_iter(), &mut out, &mut err);
            assert_eq!(code, 0);
            assert_eq!(out, help_text().as_bytes());
            assert!(err.is_empty());
        }
        for flag in ["--version", "-V"] {
            let mut out = Vec::new();
            let mut err = Vec::new();
            let code = main_with(argv(&["sigfold", flag]).into_iter(), &mut out, &mut err);
            assert_eq!(code, 0);
            assert_eq!(out, version().as_bytes());
            assert!(err.is_empty());
        }
    }

    #[test]
    #[cfg(unix)]
    fn grep_help_is_forwarded_to_grep_not_intercepted() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = main_with(
            argv(&["sigfold", "grep", "--help"]).into_iter(),
            &mut out,
            &mut err,
        );
        // grep's own --help exits 0 on GNU grep, 2 on some BSD variants —
        // either way the printed text, not the code, is what proves this
        // reached grep rather than sigfold's intercept.
        let _ = code;
        assert_ne!(
            out,
            help_text().as_bytes(),
            "sigfold's own help must not print for `sigfold grep --help`"
        );
        assert!(!out.is_empty(), "grep --help must print something");
    }

    #[test]
    #[cfg(unix)]
    fn target_command_folds_above_threshold_and_passes_through_below_it() {
        let tmp = tempfile::tempdir().unwrap();
        let fold_dir = tmp.path().join("folds");

        // Above the threshold: many matching lines force a fold.
        let big_file = tmp.path().join("big.txt");
        let mut f = std::fs::File::create(&big_file).unwrap();
        for i in 1..=40_000 {
            writeln!(f, "line {i} match").unwrap();
        }
        let cfg = FoldConfig {
            threshold_tokens: 100,
            directory: Some(fold_dir.clone()),
            ..FoldConfig::default()
        };
        // Tracking is exercised in `runner::spawn`'s own tests; disabled
        // here so this test isn't also asserting on a tracking file.
        let track = TrackConfig {
            enabled: false,
            ..TrackConfig::default()
        };
        let program = OsString::from("grep");
        let mut cmd = Command::new(&program);
        cmd.args(["match", big_file.to_str().unwrap()]);
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = dispatch_target(cmd, &program, &cfg, &track, &mut out, &mut err);
        assert_eq!(code, 0);
        let printed = String::from_utf8(out).unwrap();
        assert!(
            printed.contains("[full output: "),
            "output above the threshold must fold, got: {printed}"
        );
        assert!(err.is_empty());

        // Below the threshold: one matching line passes through untouched.
        let small_file = tmp.path().join("small.txt");
        std::fs::write(&small_file, "only match here\n").unwrap();
        let cfg = FoldConfig {
            threshold_tokens: 25_000,
            directory: Some(fold_dir),
            ..FoldConfig::default()
        };
        let mut cmd = Command::new(&program);
        cmd.args(["match", small_file.to_str().unwrap()]);
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = dispatch_target(cmd, &program, &cfg, &track, &mut out, &mut err);
        assert_eq!(code, 0);
        assert_eq!(out, b"only match here\n");
    }

    #[test]
    #[cfg(unix)]
    fn nonexistent_nontarget_program_exits_127() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        // This program's basename is not a fold target, so it exercises
        // `passthrough::exit_code_for_spawn_error` directly; the real,
        // fully-inherited-stdio version of this is in tests/cli.rs.
        let code = main_with(
            argv(&["sigfold", "sigfold-test-does-not-exist-xyz"]).into_iter(),
            &mut out,
            &mut err,
        );
        assert_eq!(code, 127);
        assert!(out.is_empty());
    }
}
