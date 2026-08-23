//! One audit row per invocation: the fixed JSONL schema and its hand-rolled
//! serialization (no `serde` — see the crate's dependency policy). Bytes
//! only, never token counts: tokens are `ceil(bytes/4)` and are derived at
//! report time, so storing both would invite drift.
//!
//! # Line atomicity
//!
//! Concurrent `brief` invocations append to the same file, and POSIX only
//! guarantees an `O_APPEND` write is atomic up to `PIPE_BUF` (4096 bytes).
//! `to_line` enforces a smaller, safer cap: the serialized line, including
//! its trailing newline, never exceeds `MAX_LINE_BYTES`. `args` is the only
//! unbounded field, so it is the one that shrinks to make the line fit.

use std::time::{SystemTime, UNIX_EPOCH};

/// Hard cap on one serialized line (including its trailing `\n`), kept
/// safely under POSIX's `PIPE_BUF` (4096 bytes) so a concurrent append from
/// another `brief` invocation can never interleave with this one.
const MAX_LINE_BYTES: usize = 4000;

/// Appended to a shrunk `args` field so the truncation is visible rather
/// than silently losing the tail of the command line.
const TRUNCATION_MARKER: &str = "...[truncated]";

/// One invocation's audit row. Flat fields only, no nested objects — a flat
/// line is cheaper to parse downstream and keeps the size bound simple.
#[derive(Debug, Clone)]
pub(crate) struct InvocationRecord {
    pub(crate) ts_ms: u128,
    pub(crate) program: String,
    pub(crate) args: String,
    /// Omitted from the line entirely when the current directory could not
    /// be read — never a fake value.
    pub(crate) cwd: Option<String>,
    pub(crate) exit_code: i32,
    pub(crate) exec_time_ms: u128,
    pub(crate) stdout_raw_bytes: usize,
    pub(crate) stdout_kept_bytes: usize,
    pub(crate) stdout_folded: bool,
    pub(crate) stdout_path: Option<String>,
    pub(crate) stderr_raw_bytes: usize,
    pub(crate) stderr_kept_bytes: usize,
    pub(crate) stderr_folded: bool,
    pub(crate) stderr_path: Option<String>,
    /// Whether any argument, resolved as a path, lies inside the resolved
    /// fold directory — see the module doc comment for what this can and
    /// cannot observe.
    pub(crate) reads_fold: bool,
}

/// Milliseconds since the Unix epoch, `0` if the clock is before it.
pub(crate) fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

impl InvocationRecord {
    /// Serialize to one JSON line, including the trailing `\n`, capped at
    /// `MAX_LINE_BYTES`. Shrinks `args` (on a UTF-8 char boundary, with a
    /// visible truncation marker) and rebuilds until the line fits.
    pub(crate) fn to_line(&self) -> String {
        let line = self.serialize(&self.args);
        if line.len() <= MAX_LINE_BYTES {
            return line;
        }

        let mut args_cap = self.args.len();
        loop {
            if args_cap == 0 {
                // Every other field is bounded (numbers, bools, short
                // paths), so this should never actually run out — but
                // tracking must never be the thing that panics.
                return self.serialize("");
            }
            let kept = truncate_utf8(&self.args, args_cap);
            let candidate = format!("{kept}{TRUNCATION_MARKER}");
            let line = self.serialize(&candidate);
            if line.len() <= MAX_LINE_BYTES {
                return line;
            }
            let over = line.len() - MAX_LINE_BYTES;
            args_cap = args_cap.saturating_sub(over.max(1));
        }
    }

    /// Build the JSON line for a given `args` value, leaving every other
    /// field untouched — the cap logic in `to_line` only ever varies this
    /// one argument.
    fn serialize(&self, args: &str) -> String {
        let mut fields: Vec<String> = Vec::with_capacity(15);
        fields.push(format!("\"ts_ms\":{}", self.ts_ms));
        fields.push(format!("\"program\":{}", json_string(&self.program)));
        fields.push(format!("\"args\":{}", json_string(args)));
        if let Some(cwd) = &self.cwd {
            fields.push(format!("\"cwd\":{}", json_string(cwd)));
        }
        fields.push(format!("\"exit_code\":{}", self.exit_code));
        fields.push(format!("\"exec_time_ms\":{}", self.exec_time_ms));
        fields.push(format!("\"stdout_raw_bytes\":{}", self.stdout_raw_bytes));
        fields.push(format!("\"stdout_kept_bytes\":{}", self.stdout_kept_bytes));
        fields.push(format!("\"stdout_folded\":{}", self.stdout_folded));
        if let Some(path) = &self.stdout_path {
            fields.push(format!("\"stdout_path\":{}", json_string(path)));
        }
        fields.push(format!("\"stderr_raw_bytes\":{}", self.stderr_raw_bytes));
        fields.push(format!("\"stderr_kept_bytes\":{}", self.stderr_kept_bytes));
        fields.push(format!("\"stderr_folded\":{}", self.stderr_folded));
        if let Some(path) = &self.stderr_path {
            fields.push(format!("\"stderr_path\":{}", json_string(path)));
        }
        fields.push(format!("\"reads_fold\":{}", self.reads_fold));
        format!("{{{}}}\n", fields.join(","))
    }
}

/// Keep at most `max_bytes` from the start of `s`, cut on a UTF-8 char
/// boundary.
fn truncate_utf8(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// JSON-quote and escape `s`: `"`, `\`, and control characters below
/// `0x20` are escaped; everything else passes through untouched.
///
/// `pub(crate)` so `report::render_json` can reuse this exact escaper
/// instead of writing a second one that could drift from what `to_line`
/// actually emits (and that `report::parse` must reverse).
pub(crate) fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

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
            stderr_raw_bytes: 0,
            stderr_kept_bytes: 0,
            stderr_folded: false,
            stderr_path: None,
            reads_fold: false,
        }
    }

    #[test]
    fn line_ends_in_newline_and_is_flat_json() {
        let line = base_record().to_line();
        assert!(line.ends_with('\n'));
        assert!(line.starts_with('{'));
        assert!(line.trim_end().ends_with('}'));
    }

    #[test]
    fn omits_cwd_when_none() {
        let mut rec = base_record();
        rec.cwd = None;
        let line = rec.to_line();
        assert!(!line.contains("\"cwd\":"));
    }

    #[test]
    fn omits_stream_paths_when_passthrough() {
        let mut rec = base_record();
        rec.stdout_folded = false;
        rec.stdout_path = None;
        let line = rec.to_line();
        assert!(!line.contains("\"stdout_path\":"));
        assert!(line.contains("\"stdout_folded\":false"));
    }

    #[test]
    fn escapes_quotes_backslashes_and_control_chars() {
        let mut rec = base_record();
        rec.args = "say \"hi\"\\there\ttab\nline".to_string();
        let line = rec.to_line();
        assert!(line.contains(r#"say \"hi\"\\there\ttab\nline"#));
    }

    #[test]
    fn escapes_other_control_chars_as_unicode_escape() {
        let mut rec = base_record();
        rec.args = "bell\u{7}".to_string();
        let line = rec.to_line();
        assert!(line.contains(r"\u0007"));
    }

    #[test]
    fn pathological_args_never_exceed_the_line_cap() {
        let mut rec = base_record();
        rec.args = "x".repeat(1_000_000);
        let line = rec.to_line();
        assert!(
            line.len() <= MAX_LINE_BYTES,
            "line was {} bytes, cap is {MAX_LINE_BYTES}",
            line.len()
        );
        assert!(
            line.contains(TRUNCATION_MARKER),
            "a shrunk args field must carry a visible truncation marker"
        );
    }

    #[test]
    fn pathological_multibyte_args_stay_under_cap_and_valid_utf8() {
        let mut rec = base_record();
        rec.args = "\u{6F22}".repeat(500_000); // 3 bytes/char
        let line = rec.to_line();
        assert!(line.len() <= MAX_LINE_BYTES);
        // The whole line must still be valid UTF-8 (String guarantees this,
        // but assert explicitly that no boundary got cut into a byte).
        assert!(std::str::from_utf8(line.as_bytes()).is_ok());
    }

    #[test]
    fn small_args_are_never_truncated() {
        let rec = base_record();
        let line = rec.to_line();
        assert!(line.contains("-r foo ."));
        assert!(!line.contains(TRUNCATION_MARKER));
    }

    #[test]
    fn now_ms_is_nonzero_at_present_day() {
        assert!(now_ms() > 0);
    }
}
