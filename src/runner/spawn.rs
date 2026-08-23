//! Spawn a child, drain both pipes through their own size gate, then write
//! each stream to its real destination — untouched bytes below the gate, a
//! compact fold above it.
//!
//! `ChildGuard` is ported verbatim from rtk's `core::stream::run_streaming`
//! (reference/rtk/src/core/stream.rs), including the reason it exists:
//! "ISSUE #897: ChildGuard RAII prevents zombie processes that caused kernel
//! panic". Every early return below — a read error, a disk error, a panicking
//! reader thread — still reaps the child through it.

use std::ffi::OsStr;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use crate::fold::paths::resolve_fold_dir;
use crate::fold::rotate::cleanup_old_files;
use crate::fold::{FoldConfig, FoldOutcome};
use crate::track::{self, InvocationRecord, TrackConfig};

use super::exit::status_to_exit_code;
use super::spill::{StreamCapture, StreamSink};

/// Result of running a command under the fold gate. stdout and stderr are
/// gated independently, so one can fold while the other passes through.
#[derive(Debug)]
pub struct RunOutcome {
    /// Exit code of the child, signal deaths reported as `128 + signal`.
    pub exit_code: i32,
    /// What happened to stdout.
    pub stdout: FoldOutcome,
    /// What happened to stderr.
    pub stderr: FoldOutcome,
}

/// ISSUE #897: ChildGuard RAII prevents zombie processes that caused kernel panic
/// (ported verbatim from rtk's `core::stream`).
struct ChildGuard(std::process::Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        self.0.wait().ok();
    }
}

/// Run `cmd` under the fold gate, writing its output to the process's own
/// stdout and stderr. Appends one best-effort tracking row per invocation —
/// see `crate::track` — governed by `track_cfg`.
pub fn run(cmd: Command, cfg: &FoldConfig, track_cfg: &TrackConfig) -> io::Result<RunOutcome> {
    let stdout = io::stdout();
    let stderr = io::stderr();
    run_with(cmd, cfg, track_cfg, &mut stdout.lock(), &mut stderr.lock())
}

/// `run` with explicit destinations, so tests can assert on the exact bytes
/// that reach each stream.
pub(crate) fn run_with(
    mut cmd: Command,
    cfg: &FoldConfig,
    track_cfg: &TrackConfig,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> io::Result<RunOutcome> {
    let slug = program_slug(&cmd);
    let dir = resolve_fold_dir(cfg);

    // stdin is inherited: a command reading a pipe or a terminal must see it
    // exactly as it would without brief. Output is piped because the gate
    // cannot decide before it has seen the bytes.
    cmd.stdin(Stdio::inherit());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let started = Instant::now();
    let mut child = ChildGuard(cmd.spawn()?);
    let child_stdout = child
        .0
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("no child stdout handle"))?;
    let child_stderr = child
        .0
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("no child stderr handle"))?;

    // Per-stream slug suffix: without it, both streams crossing the gate in
    // the same second would race for the same `{epoch}_{slug}.log` name.
    let out_sink = StreamSink::new(
        format!("{slug}.out"),
        dir.clone(),
        cfg.threshold_tokens,
        cfg.enabled,
    );
    let err_sink = StreamSink::new(
        format!("{slug}.err"),
        dir.clone(),
        cfg.threshold_tokens,
        cfg.enabled,
    );

    // Both pipes are drained concurrently. Reading one to EOF first would
    // deadlock as soon as the child filled the other pipe's buffer.
    let out_thread = std::thread::spawn(move || out_sink.pump(child_stdout));
    let err_thread = std::thread::spawn(move || err_sink.pump(child_stderr));

    // Join both threads before propagating either error: returning early on
    // the first `?` would drop the second `JoinHandle` while that thread is
    // still running, leaking a detached reader and losing whatever error it
    // hit.
    let out_capture = join_reader(out_thread, "stdout");
    let err_capture = join_reader(err_thread, "stderr");
    let out_capture = out_capture?;
    let err_capture = err_capture?;

    let status = child.0.wait()?;
    let exec_time_ms = started.elapsed().as_millis();
    let exit_code = status_to_exit_code(status);

    // Rotate once, after both fold files are complete: `cleanup_old_files`
    // deletes the oldest files, so running it while the other stream is still
    // writing could delete a file that is still being filled.
    let folded = out_capture.is_folded() || err_capture.is_folded();
    if folded && let Some(dir) = dir.as_deref() {
        cleanup_old_files(dir, cfg.max_files);
    }

    // Read the captures' sizes and paths before `emit` consumes them — this
    // is the one place a passthrough's byte count would otherwise be lost.
    let record = build_record(
        &cmd,
        dir.as_deref(),
        exit_code,
        exec_time_ms,
        &out_capture,
        &err_capture,
    );

    let stdout = emit(out_capture, out)?;
    let stderr = emit(err_capture, err)?;

    // Best-effort and always last: tracking must never delay, alter, or
    // fail the command that was just run.
    let _ = track::append(&record, track_cfg);

    Ok(RunOutcome {
        exit_code,
        stdout,
        stderr,
    })
}

/// Build the tracking row for this invocation from the command as spawned
/// (program/args, read before `emit` would otherwise be the only thing left
/// holding them) and both streams' captures (read before `emit` consumes
/// them).
fn build_record(
    cmd: &Command,
    fold_dir: Option<&Path>,
    exit_code: i32,
    exec_time_ms: u128,
    out_capture: &StreamCapture,
    err_capture: &StreamCapture,
) -> InvocationRecord {
    let program = cmd.get_program().to_string_lossy().into_owned();
    let args: Vec<String> = cmd
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let cwd = std::env::current_dir().ok();
    let reads_fold = fold_dir.is_some_and(|dir| {
        args.iter()
            .any(|arg| arg_reads_fold_dir(arg, dir, cwd.as_deref()))
    });

    InvocationRecord {
        ts_ms: track::now_ms(),
        program,
        args: args.join(" "),
        cwd: cwd.as_ref().map(|p| p.display().to_string()),
        exit_code,
        exec_time_ms,
        stdout_raw_bytes: out_capture.raw_bytes(),
        stdout_kept_bytes: out_capture.kept_bytes(),
        stdout_folded: out_capture.is_folded(),
        stdout_path: out_capture.fold_path().map(|p| p.display().to_string()),
        stderr_raw_bytes: err_capture.raw_bytes(),
        stderr_kept_bytes: err_capture.kept_bytes(),
        stderr_folded: err_capture.is_folded(),
        stderr_path: err_capture.fold_path().map(|p| p.display().to_string()),
        reads_fold,
    }
}

/// Whether `arg`, resolved as a path against `cwd`, lies inside `fold_dir`
/// — i.e. this invocation is reading back a fold file a previous invocation
/// wrote. This is inspection of the invocation's own argv, never a
/// filesystem watch or atime probe.
fn arg_reads_fold_dir(arg: &str, fold_dir: &Path, cwd: Option<&Path>) -> bool {
    let path = PathBuf::from(arg);
    let resolved = if path.is_absolute() {
        path
    } else {
        match cwd {
            Some(cwd) => cwd.join(path),
            None => return false,
        }
    };
    resolved.starts_with(fold_dir)
}

/// Write one captured stream to its destination. Passthrough bytes go out
/// raw — never through a `String`, never a lossy decode — because the child's
/// output may be binary or invalid UTF-8 and must arrive unchanged.
fn emit(capture: StreamCapture, dest: &mut dyn Write) -> io::Result<FoldOutcome> {
    match capture {
        StreamCapture::Passthrough(bytes) => {
            dest.write_all(&bytes)?;
            dest.flush()?;
            Ok(FoldOutcome::Passthrough)
        }
        StreamCapture::Folded(fold) => {
            let mut rendered = fold.render();
            rendered.push('\n');
            dest.write_all(rendered.as_bytes())?;
            dest.flush()?;
            Ok(FoldOutcome::Folded(fold))
        }
    }
}

/// Join a reader thread. A panic inside it surfaces as an error rather than
/// aborting the run — the child still gets reaped by `ChildGuard`.
fn join_reader(
    handle: std::thread::JoinHandle<io::Result<StreamCapture>>,
    stream: &str,
) -> io::Result<StreamCapture> {
    match handle.join() {
        Ok(result) => result,
        Err(_) => Err(io::Error::other(format!("{stream} reader thread panicked"))),
    }
}

/// Fold-file slug from the command's program name (basename only — a full
/// path would be sanitized into an unreadable filename).
fn program_slug(cmd: &Command) -> String {
    basename(cmd.get_program())
}

/// Basename of a program path or name. `crate::cli` reuses this to decide
/// whether `argv[1]` names one of brief's fold targets, matching exactly
/// the slug logic the fold-file writer already uses — one basename rule,
/// not two that could disagree.
pub(crate) fn basename(program: &OsStr) -> String {
    PathBuf::from(program)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "command".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(script: &str) -> Command {
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(script);
        cmd
    }

    fn cfg(dir: &std::path::Path, threshold: usize) -> FoldConfig {
        FoldConfig {
            threshold_tokens: threshold,
            directory: Some(dir.to_path_buf()),
            ..FoldConfig::default()
        }
    }

    /// Tracking disabled: these tests exercise the fold gate, not tracking,
    /// and must not write to a real tracking file.
    fn no_track() -> TrackConfig {
        TrackConfig {
            enabled: false,
            ..TrackConfig::default()
        }
    }

    /// 40k numbered lines: ~270 KB, well past the default gate.
    fn big_lines() -> String {
        (1..=40_000).map(|i| format!("line {i}\n")).collect()
    }

    #[test]
    #[cfg(unix)]
    fn binary_passthrough_is_byte_for_byte() {
        let tmp = tempfile::tempdir().unwrap();
        let mut out = Vec::new();
        let mut err = Vec::new();
        // Octal escapes: portable across dash and bash printf.
        let outcome = run_with(
            sh(r"printf '\377\376\200\001hello'"),
            &cfg(tmp.path(), 25_000),
            &no_track(),
            &mut out,
            &mut err,
        )
        .unwrap();

        assert!(matches!(outcome.stdout, FoldOutcome::Passthrough));
        assert_eq!(out, b"\xff\xfe\x80\x01hello");
        assert!(err.is_empty());
    }

    #[test]
    #[cfg(unix)]
    fn folded_run_persists_the_childs_full_output_exactly() {
        let tmp = tempfile::tempdir().unwrap();
        let mut out = Vec::new();
        let mut err = Vec::new();
        let outcome = run_with(
            sh("seq 1 40000 | sed 's/^/line /'"),
            &cfg(tmp.path(), 100),
            &no_track(),
            &mut out,
            &mut err,
        )
        .unwrap();

        let FoldOutcome::Folded(fold) = outcome.stdout else {
            panic!("expected stdout to fold");
        };
        assert_eq!(
            std::fs::read(&fold.path).unwrap(),
            big_lines().into_bytes(),
            "persisted bytes must equal the child's full output exactly"
        );
        assert_eq!(fold.total_lines, 40_000);
        // What reached the terminal is the compact summary, not the output.
        assert!(out.len() < fold.raw_bytes / 10);
        let printed = String::from_utf8(out).unwrap();
        assert!(printed.contains("line 1\n"));
        assert!(printed.contains("line 40000"));
        assert!(printed.contains("[full output: "));
    }

    #[test]
    #[cfg(unix)]
    fn binary_folded_run_persists_invalid_utf8_exactly() {
        let tmp = tempfile::tempdir().unwrap();
        let mut out = Vec::new();
        let mut err = Vec::new();
        // 200 KB of a repeated invalid-UTF-8 byte pattern.
        let outcome = run_with(
            sh(r"i=0; while [ $i -lt 4000 ]; do printf '\377\376\200\001abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuv\n'; i=$((i+1)); done"),
            &cfg(tmp.path(), 100),
            &no_track(),
            &mut out,
            &mut err,
        )
        .unwrap();

        let FoldOutcome::Folded(fold) = outcome.stdout else {
            panic!("expected stdout to fold");
        };
        let line = b"\xff\xfe\x80\x01abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuv\n";
        let expected: Vec<u8> = line
            .iter()
            .copied()
            .cycle()
            .take(line.len() * 4_000)
            .collect();
        assert_eq!(std::fs::read(&fold.path).unwrap(), expected);
        assert_eq!(fold.raw_bytes, expected.len());
    }

    #[test]
    #[cfg(unix)]
    fn stdout_folds_while_stderr_passes_through() {
        let tmp = tempfile::tempdir().unwrap();
        let mut out = Vec::new();
        let mut err = Vec::new();
        let outcome = run_with(
            sh("seq 1 40000 | sed 's/^/line /'; echo oops >&2"),
            &cfg(tmp.path(), 100),
            &no_track(),
            &mut out,
            &mut err,
        )
        .unwrap();

        assert!(matches!(outcome.stdout, FoldOutcome::Folded(_)));
        assert!(matches!(outcome.stderr, FoldOutcome::Passthrough));
        assert_eq!(err, b"oops\n", "stderr stays untouched");
    }

    #[test]
    #[cfg(unix)]
    fn stderr_folds_while_stdout_passes_through() {
        let tmp = tempfile::tempdir().unwrap();
        let mut out = Vec::new();
        let mut err = Vec::new();
        let outcome = run_with(
            sh("echo tiny; seq 1 40000 | sed 's/^/line /' >&2"),
            &cfg(tmp.path(), 100),
            &no_track(),
            &mut out,
            &mut err,
        )
        .unwrap();

        assert!(matches!(outcome.stdout, FoldOutcome::Passthrough));
        assert_eq!(out, b"tiny\n");
        let FoldOutcome::Folded(fold) = outcome.stderr else {
            panic!("expected stderr to fold");
        };
        assert_eq!(std::fs::read(&fold.path).unwrap(), big_lines().into_bytes());
        assert!(
            fold.path.to_string_lossy().contains("sh_err"),
            "stderr fold file must carry the .err suffix, got {:?}",
            fold.path
        );
    }

    #[test]
    #[cfg(unix)]
    fn both_streams_folding_get_separate_files() {
        let tmp = tempfile::tempdir().unwrap();
        let mut out = Vec::new();
        let mut err = Vec::new();
        let outcome = run_with(
            sh("seq 1 40000 | sed 's/^/line /'; seq 1 40000 | sed 's/^/line /' >&2"),
            &cfg(tmp.path(), 100),
            &no_track(),
            &mut out,
            &mut err,
        )
        .unwrap();

        let (FoldOutcome::Folded(o), FoldOutcome::Folded(e)) = (outcome.stdout, outcome.stderr)
        else {
            panic!("expected both streams to fold");
        };
        assert_ne!(o.path, e.path, "one fold file per stream, never shared");
        assert_eq!(std::fs::read(&o.path).unwrap(), big_lines().into_bytes());
        assert_eq!(std::fs::read(&e.path).unwrap(), big_lines().into_bytes());
    }

    #[test]
    #[cfg(unix)]
    fn exit_code_propagates() {
        let tmp = tempfile::tempdir().unwrap();
        let mut out = Vec::new();
        let mut err = Vec::new();
        let outcome = run_with(
            sh("exit 3"),
            &cfg(tmp.path(), 25_000),
            &no_track(),
            &mut out,
            &mut err,
        )
        .unwrap();
        assert_eq!(outcome.exit_code, 3);
    }

    #[test]
    #[cfg(unix)]
    fn exit_code_propagates_after_a_fold() {
        let tmp = tempfile::tempdir().unwrap();
        let mut out = Vec::new();
        let mut err = Vec::new();
        let outcome = run_with(
            sh("seq 1 40000 | sed 's/^/line /'; exit 2"),
            &cfg(tmp.path(), 100),
            &no_track(),
            &mut out,
            &mut err,
        )
        .unwrap();
        assert_eq!(outcome.exit_code, 2);
        assert!(matches!(outcome.stdout, FoldOutcome::Folded(_)));
    }

    #[test]
    #[cfg(unix)]
    fn empty_output_prints_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let mut out = Vec::new();
        let mut err = Vec::new();
        let outcome = run_with(
            sh("true"),
            &cfg(tmp.path(), 25_000),
            &no_track(),
            &mut out,
            &mut err,
        )
        .unwrap();

        assert_eq!(outcome.exit_code, 0);
        assert!(out.is_empty(), "no output must mean no bytes written");
        assert!(err.is_empty());
        assert!(matches!(outcome.stdout, FoldOutcome::Passthrough));
        assert!(matches!(outcome.stderr, FoldOutcome::Passthrough));
        assert_eq!(
            std::fs::read_dir(tmp.path()).unwrap().count(),
            0,
            "an empty run must not create a fold file"
        );
    }

    #[test]
    #[cfg(unix)]
    fn disabled_config_passes_large_output_through_untouched() {
        let tmp = tempfile::tempdir().unwrap();
        let mut out = Vec::new();
        let mut err = Vec::new();
        let disabled = FoldConfig {
            enabled: false,
            ..cfg(tmp.path(), 100)
        };
        let outcome = run_with(
            sh("seq 1 40000 | sed 's/^/line /'"),
            &disabled,
            &no_track(),
            &mut out,
            &mut err,
        )
        .unwrap();

        assert!(matches!(outcome.stdout, FoldOutcome::Passthrough));
        assert_eq!(out, big_lines().into_bytes());
    }

    #[test]
    #[cfg(unix)]
    fn interleaved_streams_are_never_merged() {
        let tmp = tempfile::tempdir().unwrap();
        let mut out = Vec::new();
        let mut err = Vec::new();
        run_with(
            sh("echo a; echo b >&2; echo c; echo d >&2"),
            &cfg(tmp.path(), 25_000),
            &no_track(),
            &mut out,
            &mut err,
        )
        .unwrap();

        assert_eq!(out, b"a\nc\n");
        assert_eq!(err, b"b\nd\n");
    }

    #[test]
    #[cfg(unix)]
    fn large_output_survives_a_slow_reader_without_deadlock() {
        // Both pipes fill past their kernel buffer; draining them on separate
        // threads is what keeps this from hanging.
        let tmp = tempfile::tempdir().unwrap();
        let mut out = Vec::new();
        let mut err = Vec::new();
        let outcome = run_with(
            sh("seq 1 40000 | sed 's/^/line /'; seq 1 40000 | sed 's/^/line /' >&2"),
            &cfg(tmp.path(), 25_000),
            &no_track(),
            &mut out,
            &mut err,
        )
        .unwrap();
        assert_eq!(outcome.exit_code, 0);
    }

    #[test]
    fn program_slug_uses_the_basename() {
        assert_eq!(program_slug(&Command::new("/usr/bin/grep")), "grep");
        assert_eq!(program_slug(&Command::new("cargo")), "cargo");
    }

    #[test]
    fn basename_strips_leading_path_components() {
        assert_eq!(basename(OsStr::new("/usr/bin/grep")), "grep");
        assert_eq!(basename(OsStr::new("rg")), "rg");
    }

    fn track_cfg(dir: &std::path::Path) -> TrackConfig {
        TrackConfig {
            path: Some(dir.join("tracking.jsonl")),
            ..TrackConfig::default()
        }
    }

    #[test]
    #[cfg(unix)]
    fn tracking_row_records_raw_and_kept_bytes_for_a_fold() {
        let tmp = tempfile::tempdir().unwrap();
        let mut out = Vec::new();
        let mut err = Vec::new();
        run_with(
            sh("seq 1 40000 | sed 's/^/line /'"),
            &cfg(tmp.path(), 100),
            &track_cfg(tmp.path()),
            &mut out,
            &mut err,
        )
        .unwrap();

        let line = std::fs::read_to_string(tmp.path().join("tracking.jsonl")).unwrap();
        assert_eq!(line.lines().count(), 1);
        assert!(line.contains("\"stdout_folded\":true"));
        assert!(line.contains(&format!("\"stdout_raw_bytes\":{}", big_lines().len())));
        assert!(line.contains("\"stdout_path\":"));
        assert!(line.contains("\"exit_code\":0"));
    }

    /// Passthrough rows are mandatory and carry the real byte count, never
    /// a zero placeholder — that zeroed denominator is exactly what made
    /// the tool brief replaces produce a misleading savings figure.
    #[test]
    #[cfg(unix)]
    fn tracking_row_for_passthrough_carries_real_nonzero_bytes() {
        let tmp = tempfile::tempdir().unwrap();
        let mut out = Vec::new();
        let mut err = Vec::new();
        run_with(
            sh("printf hello"),
            &cfg(tmp.path(), 25_000),
            &track_cfg(tmp.path()),
            &mut out,
            &mut err,
        )
        .unwrap();

        let line = std::fs::read_to_string(tmp.path().join("tracking.jsonl")).unwrap();
        assert!(line.contains("\"stdout_folded\":false"));
        assert!(line.contains("\"stdout_raw_bytes\":5"));
        assert!(line.contains("\"stdout_kept_bytes\":5"));
        assert!(!line.contains("\"stdout_path\":"));
    }

    #[test]
    #[cfg(unix)]
    fn tracking_disabled_writes_no_file() {
        let tmp = tempfile::tempdir().unwrap();
        let mut out = Vec::new();
        let mut err = Vec::new();
        run_with(
            sh("echo hi"),
            &cfg(tmp.path(), 25_000),
            &no_track(),
            &mut out,
            &mut err,
        )
        .unwrap();

        assert!(!tmp.path().join("tracking.jsonl").exists());
    }

    #[test]
    #[cfg(unix)]
    fn reads_fold_true_when_an_arg_resolves_inside_the_fold_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let fold_dir = tmp.path().join("folds");
        std::fs::create_dir_all(&fold_dir).unwrap();
        let fold_file = fold_dir.join("1_grep.out.log");
        std::fs::write(&fold_file, "leftover fold content\n").unwrap();

        let mut out = Vec::new();
        let mut err = Vec::new();
        let mut cmd = Command::new("cat");
        cmd.arg(&fold_file);
        run_with(
            cmd,
            &cfg(&fold_dir, 25_000),
            &track_cfg(tmp.path()),
            &mut out,
            &mut err,
        )
        .unwrap();

        let line = std::fs::read_to_string(tmp.path().join("tracking.jsonl")).unwrap();
        assert!(line.contains("\"reads_fold\":true"));
    }

    #[test]
    #[cfg(unix)]
    fn reads_fold_false_for_an_unrelated_argument() {
        let tmp = tempfile::tempdir().unwrap();
        let fold_dir = tmp.path().join("folds");
        let other = tmp.path().join("elsewhere.txt");
        std::fs::write(&other, "hi\n").unwrap();

        let mut out = Vec::new();
        let mut err = Vec::new();
        let mut cmd = Command::new("cat");
        cmd.arg(&other);
        run_with(
            cmd,
            &cfg(&fold_dir, 25_000),
            &track_cfg(tmp.path()),
            &mut out,
            &mut err,
        )
        .unwrap();

        let line = std::fs::read_to_string(tmp.path().join("tracking.jsonl")).unwrap();
        assert!(line.contains("\"reads_fold\":false"));
    }
}
