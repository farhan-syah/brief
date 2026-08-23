//! Integration tests for the compiled `sigfold` binary.
//!
//! These specifically cover behavior that only exists at the level of a
//! real OS process: the passthrough path inherits stdin/stdout/stderr
//! directly (`Stdio::inherit()`), so its output cannot be observed through
//! the in-process `out`/`err` buffers `cli::dispatch`'s unit tests use —
//! only a real child-of-a-child spawn shows it. Env vars are passed as
//! per-process `Command::env`, so unlike the crate's unit tests these never
//! race each other.

use std::process::{Command, Stdio};

fn sigfold() -> Command {
    Command::new(env!("CARGO_BIN_EXE_sigfold"))
}

#[test]
fn non_target_command_passes_through_byte_for_byte_with_its_own_exit_code() {
    let output = sigfold()
        .args(["sh", "-c", "printf 'hello\\n'; exit 7"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(7));
    assert_eq!(output.stdout, b"hello\n");
}

#[test]
fn non_target_command_is_never_folded_even_above_the_threshold() {
    // `seq`'s basename is not in sigfold's target set, so it must pass
    // through untouched no matter how low the threshold is pushed.
    let output = sigfold()
        .args(["seq", "1", "40000"])
        .env("SIGFOLD_THRESHOLD_TOKENS", "1")
        .output()
        .unwrap();
    assert!(output.status.success());
    let expected: String = (1..=40_000).map(|i| format!("{i}\n")).collect();
    assert_eq!(output.stdout, expected.as_bytes());
}

#[test]
fn nonexistent_program_exits_127() {
    let status = sigfold()
        .arg("sigfold-integration-test-does-not-exist-xyz")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(127));
}

#[cfg(unix)]
#[test]
fn non_executable_file_exits_126() {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("not-executable");
    let mut f = std::fs::File::create(&path).unwrap();
    writeln!(f, "#!/bin/sh\necho should never run").unwrap();
    drop(f);
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

    let status = sigfold()
        .arg(&path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(126));
}
