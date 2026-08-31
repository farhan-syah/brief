//! Entry point for `ogt hook`, wired from `cli::dispatch`. No flags:
//! the hook protocol is stdin-in, stdout-out, and every failure mode
//! resolves to "leave the command alone" — see `crate::hook`'s module doc.

use std::io::{Read, Write};

use super::decide::rewrite;
use super::protocol::{build_output, parse_hook_input, splice_command};

/// Read one PreToolUse payload from `stdin`, and either print nothing (the
/// default: leave the command alone) or a rewrite envelope on `out`.
/// Always returns 0 — this hook never blocks a tool call and never
/// asserts a permission decision.
pub(crate) fn run(stdin: &mut dyn Read, out: &mut dyn Write, _err: &mut dyn Write) -> i32 {
    let mut input = String::new();
    if stdin.read_to_string(&mut input).is_err() {
        return 0; // unreadable stdin: leave the command alone, never guess
    }

    let Some(hook_input) = parse_hook_input(&input) else {
        return 0;
    };
    let Some(new_command) = rewrite(&hook_input.command) else {
        return 0;
    };
    // Defensive: `parse_hook_input` already proved `tool_input_text`
    // contains a `command` field, so this should always succeed — but a
    // hook must never emit malformed output on the strength of "should."
    let Some(updated_tool_input) = splice_command(hook_input.tool_input_text, &new_command) else {
        return 0;
    };

    let _ = out.write_all(build_output(&updated_tool_input).as_bytes());
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn run_on(json: &str) -> (i32, String) {
        let mut stdin = Cursor::new(json.as_bytes());
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run(&mut stdin, &mut out, &mut err);
        (code, String::from_utf8(out).unwrap())
    }

    #[test]
    fn non_bash_tool_leaves_command_alone() {
        let json = r#"{"tool_name":"Edit","tool_input":{"file_path":"x"}}"#;
        let (code, out) = run_on(json);
        assert_eq!(code, 0);
        assert!(out.is_empty());
    }

    #[test]
    fn rewrite_target_prints_the_hookspecificoutput_envelope() {
        let json = r#"{"tool_name":"Bash","tool_input":{"command":"grep foo src/","description":"search"}}"#;
        let (code, out) = run_on(json);
        assert_eq!(code, 0);
        assert!(out.contains(r#""hookEventName":"PreToolUse""#));
        assert!(out.contains(r#""command":"ogt grep foo src/""#));
        assert!(out.contains(r#""description":"search""#));
        assert!(
            !out.contains("permissionDecision"),
            "ogt must never assert a permission decision"
        );
    }

    #[test]
    fn non_target_bash_command_leaves_alone() {
        // `echo`, not `ls` — `ls` became a fold target when the list widened,
        // and a fixture that silently turns into a target stops testing the
        // thing it is named for.
        let json = r#"{"tool_name":"Bash","tool_input":{"command":"echo hello"}}"#;
        let (code, out) = run_on(json);
        assert_eq!(code, 0);
        assert!(out.is_empty());
    }

    #[test]
    fn pipeline_is_left_alone() {
        let json = r#"{"tool_name":"Bash","tool_input":{"command":"grep foo src/ | head"}}"#;
        let (code, out) = run_on(json);
        assert_eq!(code, 0);
        assert!(out.is_empty());
    }

    #[test]
    fn malformed_json_on_stdin_leaves_command_alone() {
        let (code, out) = run_on("not json at all");
        assert_eq!(code, 0);
        assert!(out.is_empty());
    }

    #[test]
    fn already_wrapped_command_is_idempotent() {
        let json = r#"{"tool_name":"Bash","tool_input":{"command":"ogt grep foo src/"}}"#;
        let (code, out) = run_on(json);
        assert_eq!(code, 0);
        assert!(out.is_empty());
    }

    #[test]
    fn preserves_extra_tool_input_fields_verbatim() {
        let json = r#"{"tool_name":"Bash","tool_input":{"command":"cat file.txt","timeout":5,"run_in_background":false}}"#;
        let (code, out) = run_on(json);
        assert_eq!(code, 0);
        assert!(out.contains(r#""timeout":5"#));
        assert!(out.contains(r#""run_in_background":false"#));
        assert!(out.contains(r#""command":"ogt cat file.txt""#));
    }
}
