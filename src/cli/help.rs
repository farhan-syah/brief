//! sigfold's own help and version text, printed only on the intercept path
//! (see `dispatch::main_with`) — never when a fold target's own `--help`
//! reaches it, since that argv is forwarded to the child untouched.

use super::dispatch::TARGETS;

/// Oxford-comma list of `TARGETS`, e.g. "grep, cat, find, and rg" — built
/// from the same array `dispatch` matches `argv[1]` against, so the two can
/// never drift apart.
fn targets_sentence() -> String {
    match TARGETS.split_last() {
        None => String::new(),
        Some((last, [])) => (*last).to_string(),
        Some((last, rest)) => format!("{}, and {last}", rest.join(", ")),
    }
}

/// Text for `sigfold --help` / `sigfold -h`. Order is fixed: what sigfold
/// folds, the size gate, the recovery guarantee, the env overrides, usage.
pub(crate) fn help_text() -> String {
    format!(
        "\
sigfold folds output from {}; every other command runs untouched.

Folding triggers when a command's output is estimated above ~25,000 tokens.
Below that threshold, output passes through byte-for-byte, unchanged.

Folded output is never truncated: the full output is always written to disk,
and the printed hint is the exact command to read the rest.

Environment overrides:
  SIGFOLD_THRESHOLD_TOKENS   token count above which output folds (default: 25000)
  SIGFOLD_ENABLED            0 or false disables folding entirely
  SIGFOLD_FOLD_DIR           directory fold files are written to

`report` is reserved as argv[1] for `sigfold report [...]`, a falsifiable
summary of the tracking data sigfold has recorded; see `sigfold report --help`.

`hook` is reserved as argv[1] for `sigfold hook`, a Claude Code PreToolUse
hook that rewrites plain grep/cat/find/rg Bash calls to go through sigfold,
declining on anything it cannot confidently classify; see `sigfold init --help`
to install it.

`init` is reserved as argv[1] for `sigfold init [...]`, which installs or
uninstalls that hook in Claude Code's settings.json; see `sigfold init --help`.

A program literally named `report`, `hook`, or `init` must be run by path,
e.g. `sigfold ./hook`.

Usage: sigfold <program> [args...]
",
        targets_sentence()
    )
}

/// Printed on `sigfold --version` / `sigfold -V`.
pub(crate) fn version() -> String {
    format!("sigfold {}\n", env!("CARGO_PKG_VERSION"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_text_covers_targets_gate_recovery_env_usage_in_order() {
        let text = help_text();
        let targets = text.find("grep, cat, find, and rg").unwrap();
        let gate = text.find("25,000 tokens").unwrap();
        let recovery = text.find("never truncated").unwrap();
        let env = text.find("SIGFOLD_THRESHOLD_TOKENS").unwrap();
        let usage = text.find("Usage: sigfold").unwrap();
        assert!(targets < gate);
        assert!(gate < recovery);
        assert!(recovery < env);
        assert!(env < usage);
    }

    #[test]
    fn version_contains_the_crate_version() {
        assert!(version().contains(env!("CARGO_PKG_VERSION")));
    }
}
