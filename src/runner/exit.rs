//! Ported verbatim from rtk's `core::stream::status_to_exit_code`.
//! Source: reference/rtk/src/core/stream.rs

/// Exit code of a finished child: its own code, or `128 + signal` when it
/// was killed by a signal (the shell convention), or 1 when neither is
/// available.
pub fn status_to_exit_code(status: std::process::ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(sig) = status.signal() {
            return 128 + sig;
        }
    }
    1
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    #[cfg(unix)]
    fn normal_exit_code_passes_through() {
        let status = Command::new("sh").arg("-c").arg("exit 7").status().unwrap();
        assert_eq!(status_to_exit_code(status), 7);
    }

    #[test]
    #[cfg(unix)]
    fn signal_death_becomes_128_plus_signal() {
        // SIGKILL is 9 on every Unix brief targets.
        let status = Command::new("sh")
            .arg("-c")
            .arg("kill -9 $$")
            .status()
            .unwrap();
        assert_eq!(status_to_exit_code(status), 137);
    }
}
