//! brief's own help and version text, printed only on the intercept path
//! (see `dispatch::main_with`) — never when a fold target's own `--help`
//! reaches it, since that argv is forwarded to the child untouched.

use crate::targets::{oxford_list, slash_list};

/// Text for `brief --help` / `brief -h`. Order is fixed: what brief
/// folds, the size gate, the recovery guarantee, the env overrides, usage.
pub(crate) fn help_text() -> String {
    format!(
        "\
brief folds output from {}; every other command runs untouched.

Folding triggers when a command's output is estimated above ~25,000 tokens.
Below that threshold, output passes through byte-for-byte, unchanged.

Folded output is never truncated: the full output is always written to disk,
and the printed hint is the exact command to read the rest.

brief holds a command's output until it exits before deciding whether to
fold it, so a long `cargo build` or `git log` on a huge repo prints nothing
until the command finishes — see the `runner` module doc for why.

Environment overrides:
  BRIEF_THRESHOLD_TOKENS   token count above which output folds (default: 25000)
  BRIEF_ENABLED            0 or false disables folding entirely
  BRIEF_FOLD_DIR           directory fold files are written to

brief needs no integration to be useful: prefixing any command with
`brief` works from a shell, a script, or any coding agent that runs shell
commands. Nothing below is required — `hook` and `init` only automate the
prefixing for one specific harness.

`report` is reserved as argv[1] for `brief report [...]`, a falsifiable
summary of the tracking data brief has recorded; see `brief report --help`.

`hook` and `init` are the Claude Code integration, and the only harness
brief ships an adapter for. `brief hook` reads a PreToolUse payload on
stdin and rewrites plain {} Bash calls to go through
brief, declining on anything it cannot confidently classify. `brief init`
installs or uninstalls that hook in Claude Code's settings.json; see
`brief init --help`. Another harness needs its own adapter, or can simply
be told to use the `brief <program>` prefix above — or use PATH shims
instead: `brief init --shims <dir>` generates wrapper scripts that work
with any harness and any shell, no adapter required; see
`brief init --help`.

A program literally named `report`, `hook`, or `init` must be run by path,
e.g. `brief ./hook`.

Usage: brief <program> [args...]
",
        oxford_list(),
        slash_list()
    )
}

/// Printed on `brief --version` / `brief -V`.
pub(crate) fn version() -> String {
    format!("brief {}\n", env!("CARGO_PKG_VERSION"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_text_covers_targets_gate_recovery_cost_env_usage_in_order() {
        let text = help_text();
        let targets = text.find(&oxford_list()).unwrap();
        let gate = text.find("25,000 tokens").unwrap();
        let recovery = text.find("never truncated").unwrap();
        let cost = text.find("cargo build").unwrap();
        let env = text.find("BRIEF_THRESHOLD_TOKENS").unwrap();
        let usage = text.find("Usage: brief").unwrap();
        assert!(targets < gate);
        assert!(gate < recovery);
        assert!(recovery < cost);
        assert!(cost < env);
        assert!(env < usage);
    }

    #[test]
    fn help_text_names_the_hook_scope_with_the_slash_list() {
        assert!(help_text().contains(&slash_list()));
    }

    #[test]
    fn version_contains_the_crate_version() {
        assert!(version().contains(env!("CARGO_PKG_VERSION")));
    }
}
