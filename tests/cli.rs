//! Integration tests for the compiled `ogt` binary.
//!
//! These specifically cover behavior that only exists at the level of a
//! real OS process: the passthrough path inherits stdin/stdout/stderr
//! directly (`Stdio::inherit()`), so its output cannot be observed through
//! the in-process `out`/`err` buffers `cli::dispatch`'s unit tests use —
//! only a real child-of-a-child spawn shows it. Env vars are passed as
//! per-process `Command::env`, so unlike the crate's unit tests these never
//! race each other.

use std::process::{Command, Stdio};

fn ogt() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ogt"))
}

#[test]
fn non_target_command_passes_through_byte_for_byte_with_its_own_exit_code() {
    let output = ogt()
        .args(["sh", "-c", "printf 'hello\\n'; exit 7"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(7));
    assert_eq!(output.stdout, b"hello\n");
}

#[test]
fn non_target_command_is_never_folded_even_above_the_threshold() {
    // `seq`'s basename is not in ogt's target set, so it must pass
    // through untouched no matter how low the threshold is pushed.
    let output = ogt()
        .args(["seq", "1", "40000"])
        .env("OGT_THRESHOLD_TOKENS", "1")
        .output()
        .unwrap();
    assert!(output.status.success());
    let expected: String = (1..=40_000).map(|i| format!("{i}\n")).collect();
    assert_eq!(output.stdout, expected.as_bytes());
}

#[test]
fn nonexistent_program_exits_127() {
    let status = ogt()
        .arg("ogt-integration-test-does-not-exist-xyz")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(127));
}

/// Reproduces and guards the recursion bug PATH shims hit before the fix:
/// a shim `cat` on PATH exports `OGT_SHIM_DIR` and execs `ogt cat ...`;
/// without stripping the shim dir from PATH before resolving `cat`, ogt
/// would find the shim again and hang forever with zero output and no
/// tracking row. Bounded with a manual poll/kill loop (no extra crate) so
/// a regression here fails fast instead of hanging the test suite.
#[cfg(unix)]
#[test]
fn shim_does_not_recurse_into_itself() {
    use std::io::Read;
    use std::time::{Duration, Instant};

    let tmp = tempfile::tempdir().unwrap();
    let shims_dir = tmp.path().join("shims");

    let status = ogt()
        .args(["init", "--shims", shims_dir.to_str().unwrap()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "ogt init --shims must succeed");

    let shim = shims_dir.join("cat");
    assert!(shim.exists(), "cat shim must be generated");

    // The shim dir FIRST on PATH — exactly the setup that hung before the
    // fix, since PATH resolution would find the shim again.
    let real_path = std::env::var_os("PATH").unwrap_or_default();
    let mut new_path = std::ffi::OsString::from(shims_dir.as_os_str());
    new_path.push(":");
    new_path.push(&real_path);

    let file = tmp.path().join("hello.txt");
    std::fs::write(&file, "hello shim\n").unwrap();

    let mut child = Command::new(&shim)
        .arg(&file)
        .env("PATH", &new_path)
        .env_remove("OGT_SHIM_DIR")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            let mut stdout = Vec::new();
            child
                .stdout
                .take()
                .unwrap()
                .read_to_end(&mut stdout)
                .unwrap();
            assert!(status.success(), "shim exited with {status:?}");
            assert_eq!(stdout, b"hello shim\n");
            return;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!(
                "shim hung for 10s — the PATH-shim recursion guard is broken \
                 (OGT_SHIM_DIR must be stripped from PATH before resolving the program)"
            );
        }
        std::thread::sleep(Duration::from_millis(50));
    }
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

    let status = ogt()
        .arg(&path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(126));
}
