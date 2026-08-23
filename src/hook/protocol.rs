//! Hand-rolled parsing and construction for the PreToolUse hook JSON
//! contract. No `serde` (see the crate's dependency policy) — this is a
//! quote-aware, depth-aware scanner in the same spirit as
//! `report::parse`'s flat scanner, extended to track `{}`/`[]` nesting
//! because `tool_input` here is itself a nested JSON object rather than a
//! flat one.
//!
//! The one hard rule this module exists to uphold: `updatedInput` must
//! reproduce the original `tool_input` object text *verbatim*, with only
//! the `command` field's value substituted. Rebuilding the object from the
//! fields this module happens to know about would silently drop any field
//! it doesn't — so every function here returns spans into the original
//! text rather than a reconstructed value.

use crate::text_offset::offset_in;
use crate::track::json_string;

/// The two pieces pulled out of one hook stdin payload: the still-verbatim
/// `tool_input` object text (so it can be spliced, never rebuilt) and the
/// decoded `command` string (so `decide::rewrite` can classify it).
pub(crate) struct HookInput<'a> {
    pub(crate) tool_input_text: &'a str,
    pub(crate) command: String,
}

/// Parse one hook stdin payload. `None` covers every shape this hook does
/// not act on: `tool_name` isn't literally `"Bash"`, `tool_input` is
/// missing or not an object, or `command` is missing or not a JSON string
/// — each of these means "leave the command alone," never "guess."
pub(crate) fn parse_hook_input(json: &str) -> Option<HookInput<'_>> {
    let body = object_body(json)?;

    let mut tool_name = None;
    let mut tool_input_text = None;
    for field in split_top_level_commas(body) {
        let field = field.trim();
        if field.is_empty() {
            continue;
        }
        let (key_raw, value_raw) = split_key_value(field)?;
        match unquote(key_raw.trim()) {
            "tool_name" => tool_name = parse_string_value(value_raw),
            "tool_input" => tool_input_text = Some(value_raw.trim()),
            _ => {}
        }
    }

    if tool_name.as_deref() != Some("Bash") {
        return None;
    }
    let tool_input_text = tool_input_text?;
    let command = find_command(tool_input_text)?;

    Some(HookInput {
        tool_input_text,
        command,
    })
}

/// Find and decode the `command` field's value inside a `tool_input`
/// object's verbatim text.
fn find_command(tool_input_text: &str) -> Option<String> {
    let body = object_body(tool_input_text)?;
    for field in split_top_level_commas(body) {
        let field = field.trim();
        if field.is_empty() {
            continue;
        }
        let (key_raw, value_raw) = split_key_value(field)?;
        if unquote(key_raw.trim()) == "command" {
            return parse_string_value(value_raw);
        }
    }
    None
}

/// Replace `tool_input_text`'s `command` field value with `new_command`,
/// re-escaped via `json_string`, leaving every other byte — other fields,
/// field order, internal whitespace — untouched. `None` only if
/// `tool_input_text` no longer has the exact shape `parse_hook_input`
/// already validated it has, which should never happen in practice; the
/// caller must treat that as "leave the command alone," not panic.
pub(crate) fn splice_command(tool_input_text: &str, new_command: &str) -> Option<String> {
    let body = object_body(tool_input_text)?;
    for field in split_top_level_commas(body) {
        let trimmed = field.trim();
        if trimmed.is_empty() {
            continue;
        }
        let (key_raw, value_raw) = split_key_value(trimmed)?;
        if unquote(key_raw.trim()) != "command" {
            continue;
        }
        let value_trimmed = value_raw.trim();
        if !value_trimmed.starts_with('"') || !value_trimmed.ends_with('"') {
            return None;
        }
        let start = offset_in(tool_input_text, value_trimmed)?;
        let end = start + value_trimmed.len();
        let replacement = json_string(new_command);
        return Some(format!(
            "{}{}{}",
            &tool_input_text[..start],
            replacement,
            &tool_input_text[end..]
        ));
    }
    None
}

/// Build the final stdout payload for a rewrite: the fixed
/// `hookSpecificOutput` envelope around `updated_tool_input` (the already
/// spliced, still-verbatim-elsewhere `tool_input` object text).
/// `permissionDecision` is deliberately never set here — see the module
/// doc comment on `crate::hook` for why.
pub(crate) fn build_output(updated_tool_input: &str) -> String {
    format!(
        "{{\"hookSpecificOutput\":{{\"hookEventName\":\"PreToolUse\",\"updatedInput\":{updated_tool_input}}}}}\n"
    )
}

/// Strip one layer of whitespace and a matching `{`/`}` pair. `None` if
/// `s` (trimmed) doesn't have that exact shape.
fn object_body(s: &str) -> Option<&str> {
    let t = s.trim();
    let t = t.strip_prefix('{')?;
    t.strip_suffix('}')
}

/// Split a JSON object's body on its top-level commas: quote-aware (a
/// comma inside a string is never a separator) and depth-aware (a comma
/// inside a nested `{}`/`[]` value is never a separator either — the
/// difference from `report::parse::split_top_level`, which only ever sees
/// flat objects). Byte-indexed, but every byte this function acts on
/// (`"`, `\`, `,`, `{`, `}`, `[`, `]`) is ASCII, so slicing at those
/// offsets never lands inside a multi-byte UTF-8 sequence.
fn split_top_level_commas(body: &str) -> Vec<&str> {
    let bytes = body.as_bytes();
    let mut parts = Vec::new();
    let mut start = 0;
    let mut in_string = false;
    let mut escaped = false;
    let mut depth: i32 = 0;
    for (i, &b) in bytes.iter().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_string = false;
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'{' | b'[' => depth += 1,
            b'}' | b']' => depth -= 1,
            b',' if depth == 0 => {
                parts.push(&body[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&body[start..]);
    parts
}

/// Split one `"key":value` field on its first top-level colon (a colon
/// inside the key or value's own quoted string is skipped). The colon
/// separating key from value always precedes any nested `{`/`[` in the
/// value, so no depth tracking is needed here, unlike
/// `split_top_level_commas`.
fn split_key_value(field: &str) -> Option<(&str, &str)> {
    let bytes = field.as_bytes();
    let mut in_string = false;
    let mut escaped = false;
    for (i, &b) in bytes.iter().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_string = false;
            }
        } else if b == b'"' {
            in_string = true;
        } else if b == b':' {
            return Some((&field[..i], &field[i + 1..]));
        }
    }
    None
}

/// Strip one layer of surrounding `"` if present; a bare token is
/// returned unchanged.
fn unquote(s: &str) -> &str {
    s.strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(s)
}

/// Parse a JSON string value token (including its surrounding quotes) and
/// reverse `json_string`'s escaping.
fn parse_string_value(v: &str) -> Option<String> {
    let v = v.trim();
    let inner = v.strip_prefix('"')?.strip_suffix('"')?;
    unescape(inner)
}

/// Reverse of `track::json_string`'s escape set: `\"`, `\\`, `\n`, `\r`,
/// `\t`, and `\u` followed by exactly 4 hex digits. Any other escape
/// sequence is not something `json_string` can produce, so it is treated
/// as malformed rather than guessed at.
fn unescape(inner: &str) -> Option<String> {
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next()? {
            '"' => out.push('"'),
            '\\' => out.push('\\'),
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            'u' => {
                let mut hex = String::with_capacity(4);
                for _ in 0..4 {
                    hex.push(chars.next()?);
                }
                let code = u32::from_str_radix(&hex, 16).ok()?;
                out.push(char::from_u32(code)?);
            }
            _ => return None,
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_well_formed_bash_payload() {
        let json = r#"{"tool_name":"Bash","tool_input":{"command":"grep foo","description":"x"}}"#;
        let parsed = parse_hook_input(json).expect("well-formed Bash payload must parse");
        assert_eq!(parsed.command, "grep foo");
        assert_eq!(
            parsed.tool_input_text,
            r#"{"command":"grep foo","description":"x"}"#
        );
    }

    #[test]
    fn non_bash_tool_name_is_ignored() {
        let json = r#"{"tool_name":"Edit","tool_input":{"command":"grep foo"}}"#;
        assert!(parse_hook_input(json).is_none());
    }

    #[test]
    fn missing_tool_input_is_ignored() {
        let json = r#"{"tool_name":"Bash"}"#;
        assert!(parse_hook_input(json).is_none());
    }

    #[test]
    fn missing_command_is_ignored() {
        let json = r#"{"tool_name":"Bash","tool_input":{"description":"x"}}"#;
        assert!(parse_hook_input(json).is_none());
    }

    #[test]
    fn malformed_json_is_ignored() {
        assert!(parse_hook_input("not json").is_none());
        assert!(parse_hook_input("").is_none());
    }

    #[test]
    fn extra_top_level_fields_do_not_break_parsing() {
        let json = r#"{"session_id":"abc","tool_name":"Bash","tool_input":{"command":"grep foo"},"cwd":"/tmp"}"#;
        let parsed = parse_hook_input(json).expect("extra top-level fields must be tolerated");
        assert_eq!(parsed.command, "grep foo");
    }

    #[test]
    fn command_containing_json_like_text_does_not_confuse_the_scanner() {
        let json = r#"{"tool_name":"Bash","tool_input":{"command":"grep \"foo\":1,\"bar\" .","description":"x"}}"#;
        let parsed = parse_hook_input(json).expect("must parse despite JSON-shaped text in args");
        assert_eq!(parsed.command, r#"grep "foo":1,"bar" ."#);
    }

    #[test]
    fn splice_replaces_only_the_command_value() {
        let tool_input =
            r#"{"command":"grep foo","description":"a desc, with a comma","timeout":5}"#;
        let updated = splice_command(tool_input, "sigfold grep foo").expect("splice must succeed");
        assert_eq!(
            updated,
            r#"{"command":"sigfold grep foo","description":"a desc, with a comma","timeout":5}"#
        );
    }

    #[test]
    fn splice_preserves_field_order_and_unknown_fields() {
        let tool_input =
            r#"{"description":"d","command":"grep foo","weird_field":{"nested":[1,2,3]}}"#;
        let updated = splice_command(tool_input, "sigfold grep foo").unwrap();
        assert_eq!(
            updated,
            r#"{"description":"d","command":"sigfold grep foo","weird_field":{"nested":[1,2,3]}}"#
        );
    }

    #[test]
    fn build_output_shape() {
        let out = build_output(r#"{"command":"sigfold grep foo"}"#);
        assert_eq!(
            out,
            "{\"hookSpecificOutput\":{\"hookEventName\":\"PreToolUse\",\"updatedInput\":{\"command\":\"sigfold grep foo\"}}}\n"
        );
    }
}
