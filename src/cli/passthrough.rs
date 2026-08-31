//! Passthrough path for every command ogt does not fold: stdin, stdout,
//! and stderr are fully inherited from this process and the child's status
//! is translated and returned unchanged. This is not `run()` with folding
//! disabled — that would still pipe both streams and hold them until the
//! child exits, breaking tty detection, colour, and incremental progress
//! output for every one of the ~99.56% of commands ogt should never
//! observably touch.
//!
//! One thing IS observed, best-effort and after the fact: if this
//! non-target invocation's own argv names a path inside the fold directory,
//! it is the recovery read `format_full_output_hint` prescribes (`ogt
//! tail -n +N <fold file>`) — see `crate::runner::args_read_fold_dir`,
//! shared with the fold-target path so the detection logic lives in one
//! place. That row carries no measured byte counts (`captured: false`;
//! see `crate::track::InvocationRecord`) because inherited stdio is never
//! captured — inventing a byte count for it would be exactly the kind of
//! fabricated measurement this tool exists to avoid.

use std::ffi::OsStr;
use std::io::{self, Write};
use std::process::{Command, Stdio};
use std::time::Instant;

use crate::fold::FoldConfig;
use crate::fold::paths::resolve_fold_dir;
use crate::runner::{args_read_fold_dir, cmd_args_and_cwd, status_to_exit_code};
use crate::track::{self, InvocationRecord, TrackConfig};

/// Run `cmd` with all three standard streams inherited. Never pipes, never
/// buffers, never folds. Records a best-effort, always-last tracking row
/// only when this invocation's argv reads back a fold file — see the
/// module doc.
pub(crate) fn run_passthrough(
    mut cmd: Command,
    program: &OsStr,
    fold_cfg: &FoldConfig,
    track_cfg: &TrackConfig,
    err: &mut dyn Write,
) -> i32 {
    cmd.stdin(Stdio::inherit());
    cmd.stdout(Stdio::inherit());
    cmd.stderr(Stdio::inherit());

    let started = Instant::now();
    let status = match cmd.status() {
        Ok(status) => status,
        Err(io_err) => return exit_code_for_spawn_error(&io_err, err, program),
    };
    let exec_time_ms = started.elapsed().as_millis();
    let exit_code = status_to_exit_code(status);

    record_recovery_read(&cmd, fold_cfg, track_cfg, exit_code, exec_time_ms);

    exit_code
}

/// Append one tracking row for this invocation, but only when its argv
/// reads back a fold file — every other non-target command stays entirely
/// unobserved, exactly as before. Best-effort and last: a tracking failure
/// must never surface as a passthrough failure, and this runs only after
/// the child has already exited.
fn record_recovery_read(
    cmd: &Command,
    fold_cfg: &FoldConfig,
    track_cfg: &TrackConfig,
    exit_code: i32,
    exec_time_ms: u128,
) {
    let fold_dir = resolve_fold_dir(fold_cfg);
    let (args, cwd) = cmd_args_and_cwd(cmd);
    if !args_read_fold_dir(&args, fold_dir.as_deref(), cwd.as_deref()) {
        return;
    }

    let record = InvocationRecord {
        ts_ms: track::now_ms(),
        program: cmd.get_program().to_string_lossy().into_owned(),
        args: args.join(" "),
        cwd: cwd.map(|p| p.display().to_string()),
        exit_code,
        exec_time_ms,
        // Inherited stdio was never captured, so there is no measured byte
        // count for either stream — `None`, never a guessed zero.
        stdout_raw_bytes: None,
        stdout_kept_bytes: None,
        stdout_folded: false,
        stdout_path: None,
        stderr_raw_bytes: None,
        stderr_kept_bytes: None,
        stderr_folded: false,
        stderr_path: None,
        reads_fold: true,
        captured: false,
    };
    let _ = track::append(&record, track_cfg);
}

/// Map a spawn failure to the shell exit-code convention: 127 when the
/// program is not found, 126 when it exists but is not executable. Any
/// other spawn error still returns a code rather than panicking — the
/// transparency path must not be the one that breaks.
pub(crate) fn exit_code_for_spawn_error(
    io_err: &io::Error,
    err: &mut dyn Write,
    program: &OsStr,
) -> i32 {
    let code = match io_err.kind() {
        io::ErrorKind::NotFound => 127,
        io::ErrorKind::PermissionDenied => 126,
        _ => 126,
    };
    let _ = writeln!(err, "ogt: {}: {io_err}", program.to_string_lossy());
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_found_maps_to_127() {
        let io_err = io::Error::from(io::ErrorKind::NotFound);
        let mut err = Vec::new();
        let code = exit_code_for_spawn_error(&io_err, &mut err, OsStr::new("nope"));
        assert_eq!(code, 127);
        assert!(String::from_utf8(err).unwrap().contains("nope"));
    }

    #[test]
    fn permission_denied_maps_to_126() {
        let io_err = io::Error::from(io::ErrorKind::PermissionDenied);
        let mut err = Vec::new();
        let code = exit_code_for_spawn_error(&io_err, &mut err, OsStr::new("nope"));
        assert_eq!(code, 126);
    }

    fn fold_cfg(dir: &std::path::Path) -> FoldConfig {
        FoldConfig {
            directory: Some(dir.to_path_buf()),
            ..FoldConfig::default()
        }
    }

    /// Tracking disabled: these tests exercise the passthrough path itself,
    /// not tracking, and must not write to a real tracking file.
    fn no_track() -> TrackConfig {
        TrackConfig {
            enabled: false,
            ..TrackConfig::default()
        }
    }

    fn track_cfg(dir: &std::path::Path) -> TrackConfig {
        TrackConfig {
            path: Some(dir.join("tracking.jsonl")),
            ..TrackConfig::default()
        }
    }

    #[test]
    #[cfg(unix)]
    fn exit_code_propagates_through_run_passthrough() {
        let tmp = tempfile::tempdir().unwrap();
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg("exit 5");
        let mut err = Vec::new();
        let code = run_passthrough(
            cmd,
            OsStr::new("sh"),
            &fold_cfg(tmp.path()),
            &no_track(),
            &mut err,
        );
        assert_eq!(code, 5);
    }

    /// The recovery read the hint prescribes: a non-target program (`tail`
    /// is never one of `crate::targets::TARGETS`) whose argv names a
    /// path inside the fold directory must produce a tracking row, even
    /// though it ran fully passthrough.
    #[test]
    #[cfg(unix)]
    fn non_target_invocation_touching_the_fold_dir_produces_an_uncaptured_row() {
        let tmp = tempfile::tempdir().unwrap();
        let fold_dir = tmp.path().join("folds");
        std::fs::create_dir_all(&fold_dir).unwrap();
        let fold_file = fold_dir.join("1_grep.out.log");
        std::fs::write(&fold_file, "line 1\nline 2\n").unwrap();

        let mut cmd = Command::new("tail");
        cmd.arg("-n").arg("+1").arg(&fold_file);
        let mut err = Vec::new();
        run_passthrough(
            cmd,
            OsStr::new("tail"),
            &fold_cfg(&fold_dir),
            &track_cfg(tmp.path()),
            &mut err,
        );

        let line = std::fs::read_to_string(tmp.path().join("tracking.jsonl")).unwrap();
        assert_eq!(line.lines().count(), 1);
        assert!(line.contains("\"reads_fold\":true"));
        assert!(line.contains("\"captured\":false"));
        assert!(!line.contains("\"stdout_raw_bytes\":"));
        assert!(!line.contains("\"stdout_kept_bytes\":"));
    }

    /// A non-target invocation whose argv never touches the fold directory
    /// stays entirely unobserved — no row at all, not a row with
    /// `reads_fold: false`.
    #[test]
    #[cfg(unix)]
    fn non_target_invocation_not_touching_the_fold_dir_produces_no_row() {
        let tmp = tempfile::tempdir().unwrap();
        let fold_dir = tmp.path().join("folds");
        let other = tmp.path().join("elsewhere.txt");
        std::fs::write(&other, "hi\n").unwrap();

        let mut cmd = Command::new("cat");
        // Force the non-target path even though `cat` is normally a fold
        // target: `run_passthrough` itself never inspects the program name,
        // it only classifies argv against the fold directory. Dispatch is
        // what decides target vs. non-target, and that's exercised in
        // `cli::dispatch`'s own tests.
        cmd.arg(&other);
        let mut err = Vec::new();
        run_passthrough(
            cmd,
            OsStr::new("cat"),
            &fold_cfg(&fold_dir),
            &track_cfg(tmp.path()),
            &mut err,
        );

        assert!(
            !tmp.path().join("tracking.jsonl").exists(),
            "an unrelated argv must not produce a tracking file at all"
        );
    }
}
