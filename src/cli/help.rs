//! ogt's own help and version text, printed only on the intercept path
//! (see `dispatch::main_with`) — never when a fold target's own `--help`
//! reaches it, since that argv is forwarded to the child untouched.

use crate::targets::{oxford_list, slash_list};

/// Text for `ogt --help` / `ogt -h`. Order is fixed: what ogt
/// folds, the size gate, the recovery guarantee, the env overrides, usage.
pub(crate) fn help_text() -> String {
    format!(
        "\
ogt folds output from {}; every other command runs untouched.

Folding triggers when a command's output is estimated above ~25,000 tokens.
Below that threshold, output passes through byte-for-byte, unchanged.

Folded output is never truncated: the full output is always written to disk,
and the printed hint is the exact command to read the rest.

ogt holds a command's output until it exits before deciding whether to
fold it, so a long `cargo build` or `git log` on a huge repo prints nothing
until the command finishes — see the `runner` module doc for why.

Environment overrides:
  OGT_THRESHOLD_TOKENS     token count above which output folds (default: 25000)
  OGT_ENABLED              0 or false disables folding entirely
  OGT                      short alias for OGT_ENABLED; OGT_ENABLED wins
                           when both are set
  OGT_FOLD_DIR             directory fold files are written to
  OGT_ROOTS                platform-separated paths folding is scoped to
                           (overrides the roots file; see below)

Per-path scoping: ogt folds everywhere by default. To scope it to one or
more projects, list their absolute paths one per line in
<config dir>/ogt/roots (blank lines and #-comments are ignored); ogt
then folds only when the current directory is at or under a listed root.
Outside every root, folding behaves exactly like OGT_ENABLED=0 — full
passthrough, no fold file — but the invocation is still tracked, so
`ogt report` shows what scoping would have handled.

ogt needs no integration to be useful: prefixing any command with
`ogt` works from a shell, a script, or any coding agent that runs shell
commands. Nothing below is required — `hook` and `init` only automate the
prefixing for one specific harness.

`report` is reserved as argv[1] for `ogt report [...]`, a falsifiable
summary of the tracking data ogt has recorded; see `ogt report --help`.

`hook` and `init` are the Claude Code integration, and the only harness
ogt ships an adapter for. `ogt hook` reads a PreToolUse payload on
stdin and rewrites plain {} Bash calls to go through
ogt, declining on anything it cannot confidently classify. `ogt init`
installs or uninstalls that hook in Claude Code's settings.json; see
`ogt init --help`. Another harness needs its own adapter, or can simply
be told to use the `ogt <program>` prefix above — or use PATH shims
instead: `ogt init --shims <dir>` generates wrapper scripts that work
with any harness and any shell, no adapter required; see
`ogt init --help`.

A program literally named `report`, `hook`, or `init` must be run by path,
e.g. `ogt ./hook`.

Usage: ogt <program> [args...]
",
        oxford_list(),
        slash_list()
    )
}

/// Printed on `ogt --version` / `ogt -V`.
pub(crate) fn version() -> String {
    format!("ogt {}\n", env!("CARGO_PKG_VERSION"))
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
        let env = text.find("OGT_THRESHOLD_TOKENS").unwrap();
        let usage = text.find("Usage: ogt").unwrap();
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
