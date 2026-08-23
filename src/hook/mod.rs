//! `brief hook`: the PreToolUse hook Claude Code invokes before running a
//! Bash tool call. Reads one JSON payload from stdin and either exits 0
//! with empty stdout (leave the command alone) or prints a rewrite
//! envelope on stdout — see `hook_cmd` for the entry point, `protocol` for
//! the JSON contract, and `decide` for the pure rewrite rule.
//!
//! `permissionDecision` is never set: brief makes no policy judgment,
//! only a mechanical text rewrite, and lets Claude Code's native
//! ask/allow/deny flow run unmodified on the rewritten command. Asserting
//! `allow` from a tool with no permission-rule engine is how brief's
//! predecessor forced a blocking dialog with no "remember" option.

mod decide;
mod hook_cmd;
mod protocol;

pub(crate) use hook_cmd::run;
