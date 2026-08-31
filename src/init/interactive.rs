//! `ogt init`'s interactive installer, reached only when stdin is a
//! terminal, no other flag was given, and `--yes` was not passed (see
//! `cli::run_with`'s decision). Prompts on the injected `out`, reads lines
//! from the injected `reader` — never the real stdin/stdout directly, so
//! tests can drive the whole flow without a terminal.
//!
//! Nothing is written to disk until the confirm step. Any prompt that
//! hits EOF (stdin closed mid-flow) aborts immediately, writing nothing.
//! An unrecognized answer re-prompts rather than guessing.

use std::fs;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use crate::targets::TARGETS;

use super::fs_ops;
use super::shim_fs;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mechanism {
    Hook,
    Shims,
    Both,
}

impl Mechanism {
    fn wants_hook(self) -> bool {
        matches!(self, Mechanism::Hook | Mechanism::Both)
    }

    fn wants_shims(self) -> bool {
        matches!(self, Mechanism::Shims | Mechanism::Both)
    }
}

enum Scope {
    Everywhere,
    Roots(Vec<PathBuf>),
}

/// Entry point wired from `cli::run_with`.
pub(crate) fn run(
    out: &mut dyn Write,
    reader: &mut dyn BufRead,
    home_dir: Option<&Path>,
    default_shims_dir: Option<PathBuf>,
    config_dir: Option<PathBuf>,
    ogt_exe: Option<&Path>,
) -> i32 {
    let _ = writeln!(
        out,
        "ogt folds large command output to disk once it crosses a token \
         threshold, keeping a compact head/tail summary in view instead."
    );
    let _ = writeln!(out, "Nothing is written until you confirm at the end.\n");

    let Some(mechanism) = prompt_mechanism(out, reader) else {
        return abort(out);
    };

    if mechanism.wants_shims() && ogt_exe.is_none() {
        let _ = writeln!(
            out,
            "\nogt init: could not determine ogt's own executable path; \
             PATH shims cannot be installed. Nothing written."
        );
        return 1;
    }

    let shims_dir = if mechanism.wants_shims() {
        match prompt_shims_dir(out, reader, default_shims_dir.as_deref()) {
            Some(dir) => Some(dir),
            None => return abort(out),
        }
    } else {
        None
    };

    let Some(scope) = prompt_scope(out, reader) else {
        return abort(out);
    };

    if let Scope::Roots(_) = &scope
        && config_dir.is_none()
    {
        let _ = writeln!(
            out,
            "\nogt init: could not determine a config directory to write \
             the roots file to. Nothing written."
        );
        return 1;
    }

    if mechanism.wants_hook() && home_dir.is_none() {
        let _ = writeln!(
            out,
            "\nogt init: could not determine the home directory; the \
             Claude Code hook cannot be installed. Nothing written."
        );
        return 1;
    }

    print_preview(
        out,
        home_dir,
        mechanism,
        shims_dir.as_deref(),
        &scope,
        config_dir.as_deref(),
    );

    let Some(confirmed) = prompt_confirm(out, reader) else {
        return abort(out);
    };
    if !confirmed {
        let _ = writeln!(out, "\nNothing installed.");
        return 0;
    }

    install(
        out,
        home_dir,
        mechanism,
        shims_dir.as_deref(),
        &scope,
        config_dir.as_deref(),
        ogt_exe,
    )
}

fn abort(out: &mut dyn Write) -> i32 {
    let _ = writeln!(
        out,
        "\nInput ended before the flow finished; nothing was written."
    );
    1
}

/// Read one line, trimmed of its trailing newline. `None` means EOF —
/// stdin closed mid-flow, which the caller must treat as an abort.
fn read_line(reader: &mut dyn BufRead) -> Option<String> {
    let mut buf = String::new();
    match reader.read_line(&mut buf) {
        Ok(0) => None,
        Ok(_) => Some(buf.trim_end_matches(['\n', '\r']).to_string()),
        Err(_) => None,
    }
}

fn prompt_mechanism(out: &mut dyn Write, reader: &mut dyn BufRead) -> Option<Mechanism> {
    loop {
        let _ = writeln!(out, "How should ogt be installed?");
        let _ = writeln!(out, "  1) Claude Code hook");
        let _ = writeln!(out, "  2) PATH shims");
        let _ = writeln!(out, "  3) both");
        let _ = write!(out, "> ");
        let _ = out.flush();
        let line = read_line(reader)?;
        match line.trim() {
            "1" | "hook" => return Some(Mechanism::Hook),
            "2" | "shims" => return Some(Mechanism::Shims),
            "3" | "both" => return Some(Mechanism::Both),
            _ => {
                let _ = writeln!(out, "Please answer 1, 2, or 3.\n");
            }
        }
    }
}

/// Prompts for the shim directory, looping on an empty answer only when
/// there is no default to fall back to — an empty answer with a default
/// present accepts the default; an empty answer with no default is asked
/// again rather than installing to nowhere.
fn prompt_shims_dir(
    out: &mut dyn Write,
    reader: &mut dyn BufRead,
    default: Option<&Path>,
) -> Option<PathBuf> {
    loop {
        match default {
            Some(dir) => {
                let _ = writeln!(out, "\nShim directory [{}]:", dir.display());
            }
            None => {
                let _ = writeln!(
                    out,
                    "\nShim directory (no default available — enter a path):"
                );
            }
        }
        let _ = write!(out, "> ");
        let _ = out.flush();
        let line = read_line(reader)?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            match default {
                Some(dir) => return Some(dir.to_path_buf()),
                None => {
                    let _ = writeln!(out, "No default available; enter a path.");
                    continue;
                }
            }
        }
        return Some(PathBuf::from(trimmed));
    }
}

fn prompt_scope(out: &mut dyn Write, reader: &mut dyn BufRead) -> Option<Scope> {
    loop {
        let _ = writeln!(
            out,
            "\nScope: fold everywhere, or only under specific paths?"
        );
        let _ = writeln!(out, "  1) everywhere");
        let _ = writeln!(out, "  2) specific paths");
        let _ = write!(out, "> ");
        let _ = out.flush();
        let line = read_line(reader)?;
        match line.trim() {
            "1" | "everywhere" => return Some(Scope::Everywhere),
            "2" | "specific" | "paths" => {
                let roots = prompt_roots(out, reader)?;
                return Some(Scope::Roots(roots));
            }
            _ => {
                let _ = writeln!(out, "Please answer 1 or 2.");
            }
        }
    }
}

/// Reads paths one per line until a blank line. A bad entry (not
/// absolute, or does not exist) is re-prompted for rather than aborting
/// the whole flow.
fn prompt_roots(out: &mut dyn Write, reader: &mut dyn BufRead) -> Option<Vec<PathBuf>> {
    let _ = writeln!(
        out,
        "Enter one absolute path per line; an empty line finishes."
    );
    let mut roots = Vec::new();
    loop {
        let _ = write!(out, "> ");
        let _ = out.flush();
        let line = read_line(reader)?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return Some(roots);
        }
        let path = PathBuf::from(trimmed);
        if !path.is_absolute() {
            let _ = writeln!(out, "Not an absolute path: {trimmed}");
            continue;
        }
        if !path.exists() {
            let _ = writeln!(out, "Path does not exist: {}", path.display());
            continue;
        }
        roots.push(path);
    }
}

fn print_preview(
    out: &mut dyn Write,
    home_dir: Option<&Path>,
    mechanism: Mechanism,
    shims_dir: Option<&Path>,
    scope: &Scope,
    config_dir: Option<&Path>,
) {
    let _ = writeln!(out, "\nThe following will be written:");
    if mechanism.wants_hook()
        && let Some(home) = home_dir
    {
        let settings_path = home.join(".claude").join("settings.json");
        let _ = writeln!(
            out,
            "  {} (existing file backed up to {}.bak first)",
            settings_path.display(),
            settings_path.display()
        );
    }
    if let Some(dir) = shims_dir {
        for program in TARGETS {
            let _ = writeln!(out, "  {}", dir.join(program).display());
        }
    }
    match scope {
        Scope::Everywhere => {
            let _ = writeln!(out, "  (no roots file — folding stays enabled everywhere)");
        }
        Scope::Roots(roots) => {
            if let Some(cfg_dir) = config_dir {
                let roots_path = cfg_dir.join("ogt").join("roots");
                let _ = writeln!(out, "  {} with contents:", roots_path.display());
                for root in roots {
                    let _ = writeln!(out, "    {}", root.display());
                }
            }
        }
    }
    let _ = writeln!(out);
}

/// Defaults to NO: an empty answer (bare Enter), `n`, or `no` all decline.
/// Anything other than those or `y`/`yes` re-prompts.
fn prompt_confirm(out: &mut dyn Write, reader: &mut dyn BufRead) -> Option<bool> {
    loop {
        let _ = write!(out, "Install now? [y/N] ");
        let _ = out.flush();
        let line = read_line(reader)?;
        match line.trim().to_ascii_lowercase().as_str() {
            "" | "n" | "no" => return Some(false),
            "y" | "yes" => return Some(true),
            _ => {
                let _ = writeln!(out, "Please answer y or n.");
            }
        }
    }
}

/// Performs every write the confirmed flow calls for, printing the exact
/// undo command after each. Reuses `fs_ops`/`shim_fs` so the hook/shim
/// writes go through the same backup-first, atomic-write, idempotent
/// machinery as the non-interactive path.
fn install(
    out: &mut dyn Write,
    home_dir: Option<&Path>,
    mechanism: Mechanism,
    shims_dir: Option<&Path>,
    scope: &Scope,
    config_dir: Option<&Path>,
    ogt_exe: Option<&Path>,
) -> i32 {
    let mut code = 0;

    if mechanism.wants_hook() {
        let mut hook_err = Vec::new();
        let hook_code = fs_ops::run_with(home_dir, false, false, out, &mut hook_err);
        let _ = out.write_all(&hook_err);
        if hook_code != 0 {
            code = hook_code;
        } else {
            let _ = writeln!(out, "  undo: ogt init --uninstall");
        }
    }

    if let Some(dir) = shims_dir
        && let Some(ogt_exe) = ogt_exe
    {
        let mut shims_err = Vec::new();
        let shims_code = shim_fs::install_shims(dir, ogt_exe, out, &mut shims_err);
        let _ = out.write_all(&shims_err);
        if shims_code != 0 {
            code = shims_code;
        } else {
            let _ = writeln!(
                out,
                "  undo: ogt init --shims {} --uninstall",
                dir.display()
            );
        }
    }

    if let Scope::Roots(roots) = scope
        && let Some(cfg_dir) = config_dir
    {
        match write_roots_file(cfg_dir, roots) {
            Ok(path) => {
                let _ = writeln!(
                    out,
                    "  to fold everywhere again, remove: {}",
                    path.display()
                );
            }
            Err(e) => {
                let _ = writeln!(out, "ogt init: could not write the roots file: {e}");
                code = 1;
            }
        }
    }

    code
}

/// Plain write, no backup: unlike settings.json this file has no prior
/// content worth protecting — `ogt init --roots` (if ever added) or a
/// hand edit is how a user changes it later.
fn write_roots_file(config_dir: &Path, roots: &[PathBuf]) -> std::io::Result<PathBuf> {
    let dir = config_dir.join("ogt");
    fs::create_dir_all(&dir)?;
    let path = dir.join("roots");
    let mut contents = String::new();
    for root in roots {
        contents.push_str(&root.display().to_string());
        contents.push('\n');
    }
    fs::write(&path, contents)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reader(script: &str) -> impl BufRead + '_ {
        script.as_bytes()
    }

    #[test]
    fn declining_at_confirm_writes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let mut out = Vec::new();
        let mut r = reader("1\n1\n\n"); // hook, everywhere, bare Enter -> no
        let code = run(&mut out, &mut r, Some(&home), None, None, None);
        assert_eq!(code, 0);
        assert!(
            !home.join(".claude").join("settings.json").exists(),
            "declining must write nothing"
        );
        let printed = String::from_utf8(out).unwrap();
        assert!(printed.contains("Nothing installed"));
    }

    #[test]
    fn eof_mid_flow_writes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let mut out = Vec::new();
        let mut r = reader("1\n"); // mechanism only, then EOF at scope prompt
        let code = run(&mut out, &mut r, Some(&home), None, None, None);
        assert_eq!(code, 1);
        assert!(
            !home.join(".claude").exists(),
            "EOF mid-flow must write nothing"
        );
    }

    #[test]
    fn eof_at_the_very_first_prompt_writes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let mut out = Vec::new();
        let mut r = reader("");
        let code = run(&mut out, &mut r, Some(&home), None, None, None);
        assert_eq!(code, 1);
        assert!(!home.join(".claude").exists());
    }

    #[test]
    fn bad_mechanism_answer_reprompts_rather_than_aborting() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let mut out = Vec::new();
        // "banana" is unrecognized, then "1" (hook), "1" (everywhere), "n".
        let mut r = reader("banana\n1\n1\nn\n");
        let code = run(&mut out, &mut r, Some(&home), None, None, None);
        assert_eq!(code, 0);
        assert!(!home.join(".claude").join("settings.json").exists());
        let printed = String::from_utf8(out).unwrap();
        assert!(printed.contains("Please answer 1, 2, or 3"));
        assert!(printed.contains("Nothing installed"));
    }

    #[test]
    fn confirming_installs_the_hook_and_prints_the_undo_command() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let mut out = Vec::new();
        let mut r = reader("1\n1\ny\n"); // hook, everywhere, yes
        let code = run(&mut out, &mut r, Some(&home), None, None, None);
        assert_eq!(code, 0, "stdout: {}", String::from_utf8_lossy(&out));
        assert!(home.join(".claude").join("settings.json").exists());
        let printed = String::from_utf8(out).unwrap();
        assert!(printed.contains("ogt init --uninstall"));
    }

    #[test]
    fn scoped_roots_are_previewed_and_written() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let config_dir = tmp.path().join("config");
        let root = tmp.path().join("project");
        fs::create_dir_all(&root).unwrap();
        let mut out = Vec::new();
        let script = format!("1\n2\n{}\n\ny\n", root.display());
        let mut r = reader(&script); // hook, specific paths, <root>, blank, yes
        let code = run(
            &mut out,
            &mut r,
            Some(&home),
            None,
            Some(config_dir.clone()),
            None,
        );
        assert_eq!(code, 0, "stdout: {}", String::from_utf8_lossy(&out));
        let roots_file = config_dir.join("ogt").join("roots");
        assert!(roots_file.exists());
        let contents = fs::read_to_string(&roots_file).unwrap();
        assert!(contents.contains(&root.display().to_string()));
        let printed = String::from_utf8(out).unwrap();
        assert!(printed.contains(&roots_file.display().to_string()));
    }

    #[test]
    fn a_nonexistent_root_is_rejected_and_reprompted() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let config_dir = tmp.path().join("config");
        let missing = tmp.path().join("does-not-exist");
        let real = tmp.path().join("real");
        fs::create_dir_all(&real).unwrap();
        let mut out = Vec::new();
        let script = format!("1\n2\n{}\n{}\n\nn\n", missing.display(), real.display());
        let mut r = reader(&script);
        let code = run(&mut out, &mut r, Some(&home), None, Some(config_dir), None);
        assert_eq!(code, 0);
        let printed = String::from_utf8(out).unwrap();
        assert!(printed.contains("Path does not exist"));
    }

    #[test]
    fn shims_without_an_ogt_exe_refuses_before_writing_anything() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let mut out = Vec::new();
        let mut r = reader("2\n"); // shims only
        let code = run(&mut out, &mut r, Some(&home), None, None, None);
        assert_eq!(code, 1);
        assert!(!home.join(".claude").exists());
    }
}
