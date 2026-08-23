//! Passthrough path for every command brief does not fold: stdin, stdout,
//! and stderr are fully inherited from this process and the child's status
//! is translated and returned unchanged. This is not `run()` with folding
//! disabled — that would still pipe both streams and hold them until the
//! child exits, breaking tty detection, colour, and incremental progress
//! output for every one of the ~99.56% of commands brief should never
//! observably touch.

use std::ffi::OsStr;
use std::io::{self, Write};
use std::process::{Command, Stdio};

use crate::runner::status_to_exit_code;

/// Run `cmd` with all three standard streams inherited. Never pipes, never
/// buffers, never folds.
pub(crate) fn run_passthrough(mut cmd: Command, program: &OsStr, err: &mut dyn Write) -> i32 {
    cmd.stdin(Stdio::inherit());
    cmd.stdout(Stdio::inherit());
    cmd.stderr(Stdio::inherit());

    match cmd.status() {
        Ok(status) => status_to_exit_code(status),
        Err(io_err) => exit_code_for_spawn_error(&io_err, err, program),
    }
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
    let _ = writeln!(err, "brief: {}: {io_err}", program.to_string_lossy());
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

    #[test]
    fn exit_code_propagates_through_run_passthrough() {
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg("exit 5");
        let mut err = Vec::new();
        let code = run_passthrough(cmd, OsStr::new("sh"), &mut err);
        assert_eq!(code, 5);
    }
}
