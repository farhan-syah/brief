//! Parse one tracking-JSONL line into a `ReportRow`.
//!
//! This is a hand-rolled, string-aware single-pass scanner, not a naive
//! substring search for each key. `record.rs`'s `args` field carries
//! arbitrary user text — a `grep` invocation can legally search for a
//! string like `","ts_ms":1,"program":"fake` — and `json_string` escapes
//! that text's own quotes (`\"`) rather than removing them, so a correct
//! scanner must track "am I inside a quoted string" and honor `\"`/`\\`
//! exactly the way `json_string` produces them. A substring search for
//! `"ts_ms":` would find the fake one embedded in `args` instead of the
//! real top-level field.

/// One parsed tracking row, holding only the fields the report needs.
/// `args` and the fold-file paths are intentionally not parsed.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ReportRow {
    pub(crate) ts_ms: u128,
    pub(crate) program: String,
    /// `None` both when the source row omitted `cwd` (unreadable current
    /// directory at record time) and never otherwise — there is no
    /// separate "present but empty" case.
    pub(crate) cwd: Option<String>,
    pub(crate) exit_code: i32,
    pub(crate) stdout_raw_bytes: u64,
    pub(crate) stdout_kept_bytes: u64,
    pub(crate) stdout_folded: bool,
    pub(crate) stderr_raw_bytes: u64,
    pub(crate) stderr_kept_bytes: u64,
    pub(crate) stderr_folded: bool,
    pub(crate) reads_fold: bool,
}

/// Parse one JSONL line (with or without its trailing newline) into a
/// `ReportRow`. `None` means malformed: a required field is missing,
/// unparseable, or the line is not a well-formed flat JSON object — the
/// caller counts this rather than silently dropping it.
pub(crate) fn parse_line(line: &str) -> Option<ReportRow> {
    let trimmed = line.trim_end_matches(['\n', '\r']).trim();
    let body = trimmed.strip_prefix('{')?.strip_suffix('}')?;

    let mut ts_ms = None;
    let mut program = None;
    let mut cwd = None;
    let mut exit_code = None;
    let mut stdout_raw_bytes = None;
    let mut stdout_kept_bytes = None;
    let mut stdout_folded = None;
    let mut stderr_raw_bytes = None;
    let mut stderr_kept_bytes = None;
    let mut stderr_folded = None;
    let mut reads_fold = None;

    for field in split_top_level(body) {
        let field = field.trim();
        if field.is_empty() {
            continue; // the empty-object case, or a stray trailing comma
        }
        let (key_raw, value_raw) = split_key_value(field)?;
        match unquote(key_raw.trim()) {
            "ts_ms" => ts_ms = value_raw.trim().parse::<u128>().ok(),
            "program" => program = parse_string_value(value_raw),
            "cwd" => cwd = parse_string_value(value_raw),
            "exit_code" => exit_code = value_raw.trim().parse::<i32>().ok(),
            "stdout_raw_bytes" => stdout_raw_bytes = value_raw.trim().parse::<u64>().ok(),
            "stdout_kept_bytes" => stdout_kept_bytes = value_raw.trim().parse::<u64>().ok(),
            "stdout_folded" => stdout_folded = parse_bool(value_raw),
            "stderr_raw_bytes" => stderr_raw_bytes = value_raw.trim().parse::<u64>().ok(),
            "stderr_kept_bytes" => stderr_kept_bytes = value_raw.trim().parse::<u64>().ok(),
            "stderr_folded" => stderr_folded = parse_bool(value_raw),
            "reads_fold" => reads_fold = parse_bool(value_raw),
            // "args", "stdout_path", "stderr_path", or a field a future
            // schema version added: skipped, not an error.
            _ => {}
        }
    }

    Some(ReportRow {
        ts_ms: ts_ms?,
        program: program?,
        cwd,
        exit_code: exit_code?,
        stdout_raw_bytes: stdout_raw_bytes?,
        stdout_kept_bytes: stdout_kept_bytes?,
        stdout_folded: stdout_folded?,
        stderr_raw_bytes: stderr_raw_bytes?,
        stderr_kept_bytes: stderr_kept_bytes?,
        stderr_folded: stderr_folded?,
        reads_fold: reads_fold?,
    })
}

/// Split a flat JSON object's body on its top-level commas, honoring
/// quoted strings so a comma inside a string value (e.g. inside `args`)
/// is never mistaken for a field separator. Byte-indexed, but every byte
/// this function acts on (`"`, `\`, `,`) is ASCII, so slicing at those
/// offsets never lands inside a multi-byte UTF-8 sequence.
fn split_top_level(body: &str) -> Vec<&str> {
    let bytes = body.as_bytes();
    let mut parts = Vec::new();
    let mut start = 0;
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
        } else if b == b',' {
            parts.push(&body[start..i]);
            start = i + 1;
        }
    }
    parts.push(&body[start..]);
    parts
}

/// Split one `"key":value` field on its first top-level colon (a colon
/// inside the value's own quoted string is skipped, the same way
/// `split_top_level` skips a comma inside one).
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

/// Strip one layer of surrounding `"` if present; a bare token (e.g. a
/// number or `true`) is returned unchanged.
fn unquote(s: &str) -> &str {
    s.strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(s)
}

fn parse_bool(v: &str) -> Option<bool> {
    match v.trim() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// Parse a JSON string value token (including its surrounding quotes) and
/// reverse `json_string`'s escaping.
fn parse_string_value(v: &str) -> Option<String> {
    let v = v.trim();
    let inner = v.strip_prefix('"')?.strip_suffix('"')?;
    unescape(inner)
}

/// Reverse of `record::json_string`'s escape set: `\"`, `\\`, `\n`, `\r`,
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
    use crate::track::InvocationRecord;

    fn base_record() -> InvocationRecord {
        InvocationRecord {
            ts_ms: 1_700_000_000_000,
            program: "grep".to_string(),
            args: "-r foo .".to_string(),
            cwd: Some("/home/user/project".to_string()),
            exit_code: 0,
            exec_time_ms: 42,
            stdout_raw_bytes: 1000,
            stdout_kept_bytes: 200,
            stdout_folded: true,
            stdout_path: Some("/tmp/brief/folds/1_grep.log".to_string()),
            stderr_raw_bytes: 5,
            stderr_kept_bytes: 5,
            stderr_folded: false,
            stderr_path: None,
            reads_fold: false,
        }
    }

    #[test]
    fn parses_a_well_formed_line() {
        let line = base_record().to_line();
        let row = parse_line(&line).expect("well-formed line must parse");
        assert_eq!(row.ts_ms, 1_700_000_000_000);
        assert_eq!(row.program, "grep");
        assert_eq!(row.cwd.as_deref(), Some("/home/user/project"));
        assert_eq!(row.exit_code, 0);
        assert_eq!(row.stdout_raw_bytes, 1000);
        assert_eq!(row.stdout_kept_bytes, 200);
        assert!(row.stdout_folded);
        assert_eq!(row.stderr_raw_bytes, 5);
        assert_eq!(row.stderr_kept_bytes, 5);
        assert!(!row.stderr_folded);
        assert!(!row.reads_fold);
    }

    #[test]
    fn missing_cwd_parses_as_none_not_malformed() {
        let mut rec = base_record();
        rec.cwd = None;
        let row = parse_line(&rec.to_line()).expect("cwd is optional");
        assert!(row.cwd.is_none());
    }

    #[test]
    fn missing_required_field_is_malformed() {
        // Hand-written line, missing exit_code entirely.
        let line = r#"{"ts_ms":1,"program":"grep","stdout_raw_bytes":1,"stdout_kept_bytes":1,"stdout_folded":false,"stderr_raw_bytes":0,"stderr_kept_bytes":0,"stderr_folded":false,"reads_fold":false}"#;
        assert!(parse_line(line).is_none());
    }

    #[test]
    fn not_a_json_object_is_malformed() {
        assert!(parse_line("not json at all").is_none());
        assert!(parse_line("").is_none());
    }

    #[test]
    fn adversarial_args_with_fake_embedded_fields_still_parses_correctly() {
        let mut rec = base_record();
        rec.args = r#"","ts_ms":1,"program":"fake"#.to_string();
        rec.ts_ms = 42;
        rec.program = "grep".to_string();
        let line = rec.to_line();
        let row = parse_line(&line).expect("adversarial args must not break the scan");
        assert_eq!(
            row.ts_ms, 42,
            "the real ts_ms must win, not the fake one in args"
        );
        assert_eq!(
            row.program, "grep",
            "the real program must win, not the fake one in args"
        );
    }

    #[test]
    fn adversarial_args_with_backslashes_and_quotes_still_parses_correctly() {
        let mut rec = base_record();
        rec.args = r#"say \"hi\" then \\ then "unbalanced"#.to_string();
        let line = rec.to_line();
        let row =
            parse_line(&line).expect("escaped quotes/backslashes in args must not break the scan");
        assert_eq!(row.program, "grep");
    }

    #[test]
    fn truncated_args_marker_still_parses_correctly() {
        let mut rec = base_record();
        rec.args = "x".repeat(1_000_000); // forces to_line's truncation path
        let line = rec.to_line();
        assert!(line.contains("...[truncated]"));
        let row = parse_line(&line).expect("a truncated args field must still parse");
        assert_eq!(row.program, "grep");
    }

    #[test]
    fn control_character_escapes_in_program_round_trip() {
        // program is normally a bare identifier, but the parser must not
        // assume that — exercise the \u escape path via a field that does
        // support arbitrary text.
        let mut rec = base_record();
        rec.args = "bell\u{7}".to_string();
        let line = rec.to_line();
        assert!(line.contains("\\u0007"));
        let row = parse_line(&line).expect("control-char escapes must parse");
        assert_eq!(row.program, "grep");
    }

    #[test]
    fn trailing_newline_is_tolerated() {
        let line = format!("{}\n", base_record().to_line());
        assert!(parse_line(&line).is_some());
    }
}
