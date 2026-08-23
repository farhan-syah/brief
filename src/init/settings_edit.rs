//! Pure string transformations for one narrow, known JSON shape: the
//! `hooks.PreToolUse` array in a Claude Code settings.json. No filesystem
//! I/O here — `init::fs_ops` owns reading, backing up, and atomic writing.
//! No `serde` (see the crate's dependency policy): this is a depth-aware,
//! quote-aware scanner, the same technique `hook::protocol` uses, kept as
//! its own copy rather than shared since each module only ever needs to
//! understand its own narrow shape.
//!
//! # Refuse rather than guess
//!
//! Every function here returns `Err(UnrecognizedShape)` the moment the
//! text stops matching one of a small number of confidently recognized
//! shapes: no `hooks` key, a `hooks` key with no `PreToolUse` key, or a
//! `PreToolUse` key holding an array. Anything else — `hooks` or
//! `PreToolUse` holding the wrong JSON type, a malformed hook entry, a
//! settings.json that isn't a single JSON object — is refused. This edits
//! a user's live config; guessing at its shape is the one mistake this
//! module cannot recover from.
//!
//! # Byte-preserving edits
//!
//! Every insertion and removal below only ever touches the bytes of the
//! one field or array element being added or removed. This falls out of
//! `split_top_level_commas`: joining its parts with `,` always
//! reconstructs the original text exactly, so removing one part and
//! rejoining the rest reproduces every surviving byte untouched — no
//! reformatting, no re-indenting of content this module didn't create.

use crate::text_offset::offset_in;
use crate::track::json_string;

/// The exact hook-entry command this module ever installs, finds, or
/// removes. Idempotency and uninstall both key off this exact string.
const HOOK_COMMAND: &str = "brief hook";

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct UnrecognizedShape;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum InsertOutcome {
    AlreadyPresent,
    Inserted(String),
}

pub(crate) enum RemoveOutcome {
    NotPresent,
    Removed(String),
}

/// The matcher-group entry this module installs, formatted with 2-space
/// indentation. Also what `init::fs_ops` prints for the user to paste
/// manually when the file's shape isn't confidently recognized.
pub(crate) fn hook_entry_template() -> String {
    format!(
        "{{\n      \"matcher\": \"Bash\",\n      \"hooks\": [\n        {{\n          \"type\": \"command\",\n          \"command\": {}\n        }}\n      ]\n    }}",
        json_string(HOOK_COMMAND)
    )
}

/// `true` if a `PreToolUse` hook entry whose command is exactly
/// `brief hook` is already present anywhere in `settings`.
#[cfg(test)]
pub(crate) fn find_hook_entry(settings: &str) -> Result<bool, UnrecognizedShape> {
    match locate(settings)? {
        Location::Found { array_value } => array_contains_hook_command(array_value),
        _ => Ok(false),
    }
}

/// Insert the hook entry, or report it is already present. Never rewrites
/// anything outside the one field/array-element it adds.
pub(crate) fn insert_hook_entry(settings: &str) -> Result<InsertOutcome, UnrecognizedShape> {
    match locate(settings)? {
        Location::Found { array_value } => {
            if array_contains_hook_command(array_value)? {
                return Ok(InsertOutcome::AlreadyPresent);
            }
            let new_array = append_bracketed(array_value, '[', ']', &hook_entry_template());
            Ok(InsertOutcome::Inserted(splice(
                settings,
                array_value,
                &new_array,
            )?))
        }
        Location::NoPreToolUseKey { hooks_value } => {
            let field = format!("\"PreToolUse\": [\n    {}\n  ]", hook_entry_template());
            let new_hooks = append_bracketed(hooks_value, '{', '}', &field);
            Ok(InsertOutcome::Inserted(splice(
                settings,
                hooks_value,
                &new_hooks,
            )?))
        }
        Location::NoHooksKey => {
            let field = format!(
                "\"hooks\": {{\n    \"PreToolUse\": [\n      {}\n    ]\n  }}",
                hook_entry_template()
            );
            // `settings.trim()` is already validated (via `locate`'s call
            // to `top_object_body`) to be a well-formed `{...}` object, so
            // it can be handed to `append_bracketed` directly.
            Ok(InsertOutcome::Inserted(append_bracketed(
                settings.trim(),
                '{',
                '}',
                &field,
            )))
        }
    }
}

/// Remove exactly the hook-entry object whose command is `brief hook`
/// from whichever matcher group's `hooks` array holds it. Every other
/// hook, matcher group, and surrounding structure is left untouched —
/// including a now-empty `"hooks": []` on that matcher group, which this
/// function deliberately does not prune.
pub(crate) fn remove_hook_entry(settings: &str) -> Result<RemoveOutcome, UnrecognizedShape> {
    let Location::Found { array_value } = locate(settings)? else {
        return Ok(RemoveOutcome::NotPresent);
    };
    let inner = &array_value[1..array_value.len() - 1];
    let groups = split_top_level_commas(inner);

    for group in &groups {
        let g = group.trim();
        if g.is_empty() {
            continue;
        }
        if !g.starts_with('{') || !g.ends_with('}') {
            return Err(UnrecognizedShape);
        }
        let gbody = &g[1..g.len() - 1];
        let Some(hooks_val) = find_field(gbody, "hooks")? else {
            continue;
        };
        let hooks_val = hooks_val.trim();
        if !hooks_val.starts_with('[') || !hooks_val.ends_with(']') {
            return Err(UnrecognizedShape);
        }
        let hbody = &hooks_val[1..hooks_val.len() - 1];
        let mut entries = split_top_level_commas(hbody);

        for i in 0..entries.len() {
            let Some(entry_raw) = entries.get(i) else {
                continue; // unreachable given the range above, but never a panicking index
            };
            let e = entry_raw.trim();
            if e.is_empty() {
                continue;
            }
            if !e.starts_with('{') || !e.ends_with('}') {
                return Err(UnrecognizedShape);
            }
            let ebody = &e[1..e.len() - 1];
            let Some(cmd_val) = find_field(ebody, "command")? else {
                continue;
            };
            if parse_string_value(cmd_val.trim()).as_deref() != Some(HOOK_COMMAND) {
                continue;
            }

            entries.remove(i);

            // Removing the command can leave its containing matcher group
            // empty. Splicing only the inner array would strand a
            // `{"matcher": "Bash", "hooks": []}` husk in the user's config —
            // and since install always appends a fresh group, an
            // install/uninstall cycle would accumulate those husks without
            // bound. When the group has nothing left in it, drop the whole
            // group instead. A group that still holds someone else's hook is
            // never touched.
            if entries.iter().all(|e| e.trim().is_empty()) {
                let mut remaining = groups.clone();
                if let Some(pos) = remaining.iter().position(|g| std::ptr::eq(*g, *group)) {
                    remaining.remove(pos);
                }
                let new_array = format!("[{}]", remaining.join(","));
                let new_settings = splice(settings, array_value, &new_array)?;
                return Ok(RemoveOutcome::Removed(new_settings));
            }

            let new_hbody = entries.join(",");
            let new_hooks_val = format!("[{new_hbody}]");
            let new_settings = splice(settings, hooks_val, &new_hooks_val)?;
            return Ok(RemoveOutcome::Removed(new_settings));
        }
    }

    Ok(RemoveOutcome::NotPresent)
}

/// Where the `PreToolUse` array lives, or the closest recognized ancestor
/// when it doesn't exist yet — everything `insert_hook_entry` needs to
/// decide which of the three insertion shapes applies.
enum Location<'a> {
    NoHooksKey,
    NoPreToolUseKey { hooks_value: &'a str },
    Found { array_value: &'a str },
}

fn locate(settings: &str) -> Result<Location<'_>, UnrecognizedShape> {
    let top_body = top_object_body(settings)?;
    let Some(hooks_value) = find_field(top_body, "hooks")? else {
        return Ok(Location::NoHooksKey);
    };
    let hooks_value = hooks_value.trim();
    if !hooks_value.starts_with('{') || !hooks_value.ends_with('}') {
        return Err(UnrecognizedShape);
    }
    let hooks_body = &hooks_value[1..hooks_value.len() - 1];
    let Some(ptu_value) = find_field(hooks_body, "PreToolUse")? else {
        return Ok(Location::NoPreToolUseKey { hooks_value });
    };
    let ptu_value = ptu_value.trim();
    if !ptu_value.starts_with('[') || !ptu_value.ends_with(']') {
        return Err(UnrecognizedShape);
    }
    Ok(Location::Found {
        array_value: ptu_value,
    })
}

/// Scan every matcher group in a `PreToolUse` array's `[...]` text for a
/// hook entry whose command is exactly `brief hook`.
fn array_contains_hook_command(array_value: &str) -> Result<bool, UnrecognizedShape> {
    let inner = &array_value[1..array_value.len() - 1];
    for group in split_top_level_commas(inner) {
        let g = group.trim();
        if g.is_empty() {
            continue;
        }
        if !g.starts_with('{') || !g.ends_with('}') {
            return Err(UnrecognizedShape);
        }
        let gbody = &g[1..g.len() - 1];
        let Some(hooks_val) = find_field(gbody, "hooks")? else {
            continue;
        };
        let hooks_val = hooks_val.trim();
        if !hooks_val.starts_with('[') || !hooks_val.ends_with(']') {
            return Err(UnrecognizedShape);
        }
        let hbody = &hooks_val[1..hooks_val.len() - 1];
        for entry in split_top_level_commas(hbody) {
            let e = entry.trim();
            if e.is_empty() {
                continue;
            }
            if !e.starts_with('{') || !e.ends_with('}') {
                return Err(UnrecognizedShape);
            }
            let ebody = &e[1..e.len() - 1];
            if let Some(cmd_val) = find_field(ebody, "command")?
                && parse_string_value(cmd_val.trim()).as_deref() == Some(HOOK_COMMAND)
            {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Insert `new_text` as a new trailing field/element inside `value`
/// (which includes its own `open`/`close` delimiters), preserving every
/// existing byte — only the whitespace immediately before `close` (pure
/// indentation, never content) is trimmed away so the addition lands on
/// its own line.
fn append_bracketed(value: &str, open: char, close: char, new_text: &str) -> String {
    let inner = &value[open.len_utf8()..value.len() - close.len_utf8()];
    if inner.trim().is_empty() {
        format!("{open}\n  {new_text}\n{close}")
    } else {
        let trimmed_trailing = inner.trim_end_matches([' ', '\t', '\n', '\r']);
        format!("{open}{trimmed_trailing},\n  {new_text}\n{close}")
    }
}

/// Replace `target` (a subslice of `parent`) with `replacement`, everywhere
/// else in `parent` untouched.
fn splice(parent: &str, target: &str, replacement: &str) -> Result<String, UnrecognizedShape> {
    let start = offset_in(parent, target).ok_or(UnrecognizedShape)?;
    let end = start + target.len();
    Ok(format!(
        "{}{}{}",
        &parent[..start],
        replacement,
        &parent[end..]
    ))
}

/// Strip one layer of whitespace and a matching `{`/`}` pair. `None` if
/// `s` (trimmed) doesn't have that exact shape.
fn top_object_body(s: &str) -> Result<&str, UnrecognizedShape> {
    let t = s.trim();
    let inner = t.strip_prefix('{').ok_or(UnrecognizedShape)?;
    inner.strip_suffix('}').ok_or(UnrecognizedShape)
}

/// Find `key`'s value within a flat-or-nested JSON object body's top-level
/// fields. `Ok(None)` means the key is genuinely absent; `Err` means a
/// field couldn't be split into `key:value` at all, which is not the same
/// thing and must not be treated as "absent."
fn find_field<'a>(body: &'a str, key: &str) -> Result<Option<&'a str>, UnrecognizedShape> {
    for field in split_top_level_commas(body) {
        let field = field.trim();
        if field.is_empty() {
            continue;
        }
        let (k, v) = split_key_value(field).ok_or(UnrecognizedShape)?;
        if unquote(k.trim()) == key {
            return Ok(Some(v.trim()));
        }
    }
    Ok(None)
}

/// Split a JSON object's body on its top-level commas: quote-aware (a
/// comma inside a string is never a separator) and depth-aware (a comma
/// inside a nested `{}`/`[]` value is never a separator either). Joining
/// the returned parts with `,` always reproduces `body` exactly — this
/// property is what makes `remove_hook_entry`'s reconstruction
/// byte-preserving. Byte-indexed, but every byte this function acts on
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
/// value, so no depth tracking is needed here.
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
/// reverse `track::json_string`'s escaping.
fn parse_string_value(v: &str) -> Option<String> {
    let inner = v.strip_prefix('"')?.strip_suffix('"')?;
    unescape(inner)
}

/// Reverse of `track::json_string`'s escape set: `\"`, `\\`, `\n`, `\r`,
/// `\t`, and `\u` followed by exactly 4 hex digits.
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
    fn no_hooks_key_installs_fresh_structure() {
        let settings = "{}";
        let InsertOutcome::Inserted(new) = insert_hook_entry(settings).unwrap() else {
            panic!("expected Inserted");
        };
        assert!(new.contains("\"hooks\""));
        assert!(new.contains("\"PreToolUse\""));
        assert!(new.contains("\"matcher\": \"Bash\""));
        assert!(new.contains("\"command\": \"brief hook\""));
        assert!(find_hook_entry(&new).unwrap());
    }

    #[test]
    fn no_hooks_key_preserves_other_top_level_fields() {
        let settings = r#"{"env":{"FOO":"bar"},"otherField":123}"#;
        let InsertOutcome::Inserted(new) = insert_hook_entry(settings).unwrap() else {
            panic!("expected Inserted");
        };
        assert!(new.contains(r#""env":{"FOO":"bar"}"#));
        assert!(new.contains(r#""otherField":123"#));
        assert!(find_hook_entry(&new).unwrap());
    }

    #[test]
    fn hooks_key_without_pre_tool_use_adds_it() {
        let settings = r#"{"hooks":{"PostToolUse":[{"matcher":"*","hooks":[]}]}}"#;
        let InsertOutcome::Inserted(new) = insert_hook_entry(settings).unwrap() else {
            panic!("expected Inserted");
        };
        assert!(new.contains(r#""PostToolUse":[{"matcher":"*","hooks":[]}]"#));
        assert!(new.contains("\"PreToolUse\""));
        assert!(find_hook_entry(&new).unwrap());
    }

    #[test]
    fn existing_pre_tool_use_array_gets_new_matcher_group_appended() {
        let settings = r#"{"hooks":{"PreToolUse":[{"matcher":"Edit","hooks":[{"type":"command","command":"lint.sh"}]}]}}"#;
        let InsertOutcome::Inserted(new) = insert_hook_entry(settings).unwrap() else {
            panic!("expected Inserted");
        };
        assert!(new.contains(r#""matcher":"Edit""#));
        assert!(new.contains("lint.sh"));
        assert!(find_hook_entry(&new).unwrap());
    }

    #[test]
    fn empty_pre_tool_use_array_gets_first_element() {
        let settings = r#"{"hooks":{"PreToolUse":[]}}"#;
        let InsertOutcome::Inserted(new) = insert_hook_entry(settings).unwrap() else {
            panic!("expected Inserted");
        };
        assert!(find_hook_entry(&new).unwrap());
    }

    #[test]
    fn already_present_is_reported_and_nothing_changes() {
        let settings = r#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"brief hook"}]}]}}"#;
        match insert_hook_entry(settings).unwrap() {
            InsertOutcome::AlreadyPresent => {}
            InsertOutcome::Inserted(_) => panic!("must be idempotent"),
        }
    }

    #[test]
    fn present_among_other_matcher_groups_is_still_detected() {
        let settings = r#"{"hooks":{"PreToolUse":[{"matcher":"Edit","hooks":[{"type":"command","command":"lint.sh"}]},{"matcher":"Bash","hooks":[{"type":"command","command":"brief hook"}]}]}}"#;
        assert!(find_hook_entry(settings).unwrap());
        match insert_hook_entry(settings).unwrap() {
            InsertOutcome::AlreadyPresent => {}
            InsertOutcome::Inserted(_) => panic!("must be idempotent"),
        }
    }

    #[test]
    fn hooks_not_an_object_is_unrecognized() {
        let settings = r#"{"hooks":"not an object"}"#;
        assert_eq!(insert_hook_entry(settings), Err(UnrecognizedShape));
    }

    #[test]
    fn pre_tool_use_not_an_array_is_unrecognized() {
        let settings = r#"{"hooks":{"PreToolUse":"not an array"}}"#;
        assert_eq!(insert_hook_entry(settings), Err(UnrecognizedShape));
    }

    #[test]
    fn not_a_json_object_at_all_is_unrecognized() {
        assert_eq!(insert_hook_entry("not json"), Err(UnrecognizedShape));
        assert_eq!(insert_hook_entry("[1,2,3]"), Err(UnrecognizedShape));
    }

    #[test]
    fn removing_the_only_entry_drops_the_empty_matcher_group() {
        // Install always appends a fresh matcher group, so leaving an empty
        // husk behind would accumulate one per install/uninstall cycle.
        let settings = "{\n  \"hooks\": {\n    \"PreToolUse\": []\n  }\n}";
        let installed = match insert_hook_entry(settings).unwrap() {
            InsertOutcome::Inserted(text) => text,
            InsertOutcome::AlreadyPresent => panic!("fixture should not already have it"),
        };
        let removed = match remove_hook_entry(&installed).unwrap() {
            RemoveOutcome::Removed(text) => text,
            RemoveOutcome::NotPresent => panic!("just installed it"),
        };
        assert!(
            !removed.contains("matcher"),
            "the emptied matcher group must be dropped, got: {removed}"
        );
        assert!(!removed.contains("brief hook"));

        // And the cycle must converge: installing again yields exactly the
        // same text as the first install, never an accumulation.
        let reinstalled = match insert_hook_entry(&removed).unwrap() {
            InsertOutcome::Inserted(text) => text,
            InsertOutcome::AlreadyPresent => panic!("it was removed"),
        };
        assert_eq!(reinstalled, installed, "install/uninstall must converge");
    }

    #[test]
    fn removing_ours_keeps_a_foreign_hook_in_the_same_group() {
        let settings = "{\n  \"hooks\": {\n    \"PreToolUse\": [\n      {\"matcher\": \"Bash\", \"hooks\": [\n        {\"type\": \"command\", \"command\": \"brief hook\"},\n        {\"type\": \"command\", \"command\": \"their-tool\"}\n      ]}\n    ]\n  }\n}";
        let removed = match remove_hook_entry(settings).unwrap() {
            RemoveOutcome::Removed(text) => text,
            RemoveOutcome::NotPresent => panic!("it is present"),
        };
        assert!(!removed.contains("brief hook"));
        assert!(
            removed.contains("their-tool"),
            "a foreign hook sharing the group must survive"
        );
        assert!(
            removed.contains("matcher"),
            "the group must stay, it still holds someone else's hook"
        );
    }

    #[test]
    fn remove_deletes_only_the_brief_entry() {
        let settings = r#"{"hooks":{"PreToolUse":[{"matcher":"Edit","hooks":[{"type":"command","command":"lint.sh"}]},{"matcher":"Bash","hooks":[{"type":"command","command":"brief hook"}]}]}}"#;
        let RemoveOutcome::Removed(new) = remove_hook_entry(settings).unwrap() else {
            panic!("expected Removed");
        };
        assert!(new.contains("lint.sh"), "other matcher group must survive");
        assert!(!new.contains("brief hook"));
        assert!(!find_hook_entry(&new).unwrap());
    }

    #[test]
    fn remove_leaves_other_hooks_in_the_same_matcher_group_untouched() {
        let settings = r#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"lint.sh"},{"type":"command","command":"brief hook"}]}]}}"#;
        let RemoveOutcome::Removed(new) = remove_hook_entry(settings).unwrap() else {
            panic!("expected Removed");
        };
        assert!(new.contains("lint.sh"));
        assert!(!new.contains("brief hook"));
    }

    #[test]
    fn remove_prunes_the_matcher_group_it_empties() {
        let settings = r#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"brief hook"}]}]}}"#;
        let RemoveOutcome::Removed(new) = remove_hook_entry(settings).unwrap() else {
            panic!("expected Removed");
        };
        assert!(!new.contains("brief hook"));
        assert!(
            !new.contains(r#""matcher":"Bash""#),
            "an emptied group must not be left behind as a husk, got: {new}"
        );
    }

    #[test]
    fn remove_when_absent_is_a_no_op() {
        let settings = r#"{"hooks":{"PreToolUse":[{"matcher":"Edit","hooks":[{"type":"command","command":"lint.sh"}]}]}}"#;
        match remove_hook_entry(settings).unwrap() {
            RemoveOutcome::NotPresent => {}
            RemoveOutcome::Removed(_) => panic!("nothing to remove"),
        }
    }

    #[test]
    fn remove_with_no_hooks_key_is_a_no_op() {
        match remove_hook_entry("{}").unwrap() {
            RemoveOutcome::NotPresent => {}
            RemoveOutcome::Removed(_) => panic!("nothing to remove"),
        }
    }

    #[test]
    fn comma_inside_a_string_value_never_splits_a_field() {
        let settings = r#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"brief hook","description":"a, b, c"}]}]}}"#;
        assert!(find_hook_entry(settings).unwrap());
        let RemoveOutcome::Removed(new) = remove_hook_entry(settings).unwrap() else {
            panic!("expected Removed");
        };
        // The point of the fixture is the commas inside `description`: a
        // splitter that ignored quoting would tear that field apart and
        // fail to match the command beside it.
        assert!(!new.contains("brief hook"));
        assert!(!new.contains("a, b, c"));
    }
}
