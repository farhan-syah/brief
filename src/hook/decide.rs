//! The rewrite rule: pure text classification of one shell command string,
//! no I/O, no knowledge of the hook protocol that wraps it.
//!
//! # Why declining is the default, not a fallback
//!
//! sigfold's predecessor rewrote commands it did not fully understand and
//! was lossy on several, so users learned to bypass it before ever seeing
//! its output. A hook that changes what a command *means* destroys trust;
//! one that declines to act is merely less useful. Every ambiguous case
//! here resolves toward `None` — leave the command alone.

/// The only programs this hook ever rewrites, matched on the first
/// remaining word's basename. Mirrors `crate::cli::dispatch::TARGETS` but
/// is kept as its own list: the hook's classification is deliberately
/// independent of dispatch's fold-target wiring, even though the values
/// happen to match today.
const TARGETS: [&str; 4] = ["grep", "cat", "find", "rg"];

/// Classify `cmd` and return its rewrite, or `None` to leave it alone.
///
/// Pure and total: never panics, never blocks, and only ever inspects the
/// text handed to it. See the module doc for why `None` is the default.
pub(crate) fn rewrite(cmd: &str) -> Option<String> {
    let words = scan_words(cmd)?;

    let mut idx = 0;
    while idx < words.len() {
        let (start, end) = words[idx];
        if is_assignment(&cmd[start..end]) {
            idx += 1;
            continue;
        }
        break;
    }
    let (prog_start, prog_end) = *words.get(idx)?;
    let raw_prog = &cmd[prog_start..prog_end];
    let prog = dequote(raw_prog);

    // Privilege context is not something to wrap on a 0.44% case.
    if prog == "sudo" {
        return None;
    }

    let base = basename_of(&prog);
    if !TARGETS.contains(&base) {
        // Also covers the idempotent case (`sigfold grep ...` — the first
        // word's basename is "sigfold", never one of `TARGETS`) and
        // `echo grep`/`man grep`/`xargs grep` (the first word is the
        // wrapper, never the target itself).
        return None;
    }

    Some(format!(
        "{}sigfold {}",
        &cmd[..prog_start],
        &cmd[prog_start..]
    ))
}

/// Scan `cmd` for unquoted shell metacharacters and, in the same pass,
/// split it into whitespace-delimited word spans (byte offsets into `cmd`).
/// `None` means "reject": an unquoted metacharacter was found, a quote was
/// left open, or the command ends in a line-continuation backslash — any of
/// which makes the command unclassifiable, so the caller must decline.
///
/// Quote-awareness is the load-bearing part of this scan: a metacharacter
/// is only ever a *reject* signal outside both single and double quotes.
/// `grep -rn 'a|b' src/` and `grep -rn "a|b" src/` must both survive; only
/// a genuinely unquoted `|`/`;`/`&`/newline/backtick/`$(`/`<(`/`>(`/`<<`
/// rejects.
fn scan_words(cmd: &str) -> Option<Vec<(usize, usize)>> {
    let chars: Vec<(usize, char)> = cmd.char_indices().collect();
    let n = chars.len();

    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;
    let mut word_start: Option<usize> = None;
    let mut words = Vec::new();

    let mut i = 0;
    while i < n {
        let Some(&(idx, ch)) = chars.get(i) else {
            break; // unreachable given `i < n`, but never a panicking index
        };

        if escaped {
            escaped = false;
            i += 1;
            continue;
        }

        if in_single {
            if ch == '\'' {
                in_single = false;
            }
            i += 1;
            continue;
        }

        if in_double {
            if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_double = false;
            }
            i += 1;
            continue;
        }

        // Top level: not inside any quote, not escaped.
        match ch {
            '\'' => {
                word_start.get_or_insert(idx);
                in_single = true;
            }
            '"' => {
                word_start.get_or_insert(idx);
                in_double = true;
            }
            '\\' => {
                if i + 1 == n {
                    return None; // trailing line-continuation backslash
                }
                word_start.get_or_insert(idx);
                escaped = true;
            }
            '\n' | '|' | ';' | '&' | '`' => return None,
            '$' if next_is(&chars, i, '(') => return None,
            '<' if next_is(&chars, i, '(') || next_is(&chars, i, '<') => return None,
            '>' if next_is(&chars, i, '(') => return None,
            ' ' | '\t' => {
                if let Some(s) = word_start.take() {
                    words.push((s, idx));
                }
            }
            _ => {
                word_start.get_or_insert(idx);
            }
        }
        i += 1;
    }

    if in_single || in_double {
        return None; // unterminated quote: unclassifiable, so decline
    }
    if let Some(s) = word_start.take() {
        words.push((s, cmd.len()));
    }
    Some(words)
}

fn next_is(chars: &[(usize, char)], i: usize, want: char) -> bool {
    chars.get(i + 1).is_some_and(|&(_, c)| c == want)
}

/// `true` if `raw` (a word span's untouched source text) is a leading
/// `VAR=val` shell assignment: `VAR` is a valid POSIX identifier and `=`
/// appears before any quote character opens.
fn is_assignment(raw: &str) -> bool {
    let Some(eq) = raw.find('=') else {
        return false;
    };
    let name = &raw[..eq];
    if name.is_empty() {
        return false;
    }
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Strip quote markers from one already-validated word span and resolve
/// backslash escapes, the same way a shell would before exec — needed only
/// to classify the program name, never to alter what gets rewritten (the
/// substitution in `rewrite` splices the original, still-quoted text back
/// in untouched).
fn dequote(word: &str) -> String {
    let mut out = String::with_capacity(word.len());
    let mut in_single = false;
    let mut in_double = false;
    let mut chars = word.chars();
    while let Some(c) = chars.next() {
        if in_single {
            if c == '\'' {
                in_single = false;
            } else {
                out.push(c);
            }
            continue;
        }
        if in_double {
            if c == '\\' {
                if let Some(nc) = chars.next() {
                    out.push(nc);
                }
            } else if c == '"' {
                in_double = false;
            } else {
                out.push(c);
            }
            continue;
        }
        match c {
            '\'' => in_single = true,
            '"' => in_double = true,
            '\\' => {
                if let Some(nc) = chars.next() {
                    out.push(nc);
                }
            }
            _ => out.push(c),
        }
    }
    out
}

/// The path component after the last `/`, or the whole string if there is
/// none — mirrors `crate::runner::basename`'s semantics but works on a
/// dequoted `&str` rather than an `OsStr` argv element.
fn basename_of(s: &str) -> &str {
    s.rsplit('/').next().unwrap_or(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_grep_is_rewritten() {
        assert_eq!(
            rewrite("grep foo src/"),
            Some("sigfold grep foo src/".to_string())
        );
    }

    #[test]
    fn pipe_inside_single_quotes_is_rewritten() {
        assert_eq!(
            rewrite("grep -rn 'a|b' src/"),
            Some("sigfold grep -rn 'a|b' src/".to_string())
        );
    }

    #[test]
    fn pipe_inside_double_quotes_is_rewritten() {
        assert_eq!(
            rewrite("grep -rn \"a|b\" src/"),
            Some("sigfold grep -rn \"a|b\" src/".to_string())
        );
    }

    #[test]
    fn backslash_escaped_pipe_is_rewritten() {
        assert_eq!(
            rewrite(r"grep -rn a\|b src/"),
            Some(r"sigfold grep -rn a\|b src/".to_string())
        );
    }

    #[test]
    fn the_measured_worst_case_regex_is_rewritten() {
        // The single largest command in the measured history: a `|`
        // alternation regex inside single quotes. A naive `contains('|')`
        // check would misclassify this as a pipeline.
        let cmd = "grep -rn 'image|generate_image|gpt-image|dall|tool' /path";
        assert_eq!(rewrite(cmd), Some(format!("sigfold {cmd}")));
    }

    #[test]
    fn genuine_pipe_is_left_alone() {
        assert_eq!(rewrite("grep foo src/ | head"), None);
    }

    #[test]
    fn double_ampersand_is_left_alone() {
        assert_eq!(rewrite("grep foo && echo hi"), None);
    }

    #[test]
    fn double_pipe_is_left_alone() {
        assert_eq!(rewrite("grep foo || echo hi"), None);
    }

    #[test]
    fn semicolon_is_left_alone() {
        assert_eq!(rewrite("grep foo; echo hi"), None);
    }

    #[test]
    fn backtick_is_left_alone() {
        assert_eq!(rewrite("grep `whoami`"), None);
    }

    #[test]
    fn command_substitution_is_left_alone() {
        assert_eq!(rewrite("grep $(whoami)"), None);
    }

    #[test]
    fn process_substitution_is_left_alone() {
        assert_eq!(rewrite("grep <(cat a) b"), None);
        assert_eq!(rewrite("grep foo >(cat)"), None);
    }

    #[test]
    fn heredoc_is_left_alone() {
        assert_eq!(rewrite("grep foo <<EOF"), None);
    }

    #[test]
    fn newline_is_left_alone() {
        assert_eq!(rewrite("grep foo\necho hi"), None);
    }

    #[test]
    fn trailing_backslash_continuation_is_left_alone() {
        assert_eq!(rewrite("grep foo \\"), None);
    }

    #[test]
    fn unterminated_single_quote_is_left_alone() {
        assert_eq!(rewrite("grep -rn 'unclosed src/"), None);
    }

    #[test]
    fn unterminated_double_quote_is_left_alone() {
        assert_eq!(rewrite("grep -rn \"unclosed src/"), None);
    }

    #[test]
    fn sudo_grep_is_left_alone() {
        assert_eq!(rewrite("sudo grep foo src/"), None);
    }

    #[test]
    fn echo_grep_never_matches() {
        assert_eq!(rewrite("echo grep"), None);
    }

    #[test]
    fn man_grep_never_matches() {
        assert_eq!(rewrite("man grep"), None);
    }

    #[test]
    fn xargs_grep_never_matches() {
        assert_eq!(rewrite("xargs grep foo"), None);
    }

    #[test]
    fn already_sigfold_wrapped_is_idempotent() {
        assert_eq!(rewrite("sigfold grep foo src/"), None);
    }

    #[test]
    fn non_target_command_is_left_alone() {
        assert_eq!(rewrite("ls -la"), None);
    }

    #[test]
    fn absolute_path_basename_is_rewritten_preserving_the_path() {
        assert_eq!(
            rewrite("/usr/bin/grep foo src/"),
            Some("sigfold /usr/bin/grep foo src/".to_string())
        );
    }

    #[test]
    fn leading_var_assignment_is_skipped_to_find_the_program() {
        // `sigfold` goes AFTER the assignments, not before them. The shell
        // applies a leading `VAR=val` to the environment of the command it
        // prefixes, and sigfold's child inherits it — so the variable still
        // reaches grep. Putting `sigfold` first instead would make it try to
        // execute a program literally named `FOO=bar`.
        assert_eq!(
            rewrite("FOO=bar grep foo src/"),
            Some("FOO=bar sigfold grep foo src/".to_string())
        );
    }

    #[test]
    fn multiple_leading_assignments_are_skipped() {
        assert_eq!(
            rewrite("A=1 B=2 rg foo"),
            Some("A=1 B=2 sigfold rg foo".to_string())
        );
    }

    #[test]
    fn only_assignments_no_program_is_left_alone() {
        assert_eq!(rewrite("FOO=bar"), None);
    }

    #[test]
    fn empty_command_is_left_alone() {
        assert_eq!(rewrite(""), None);
        assert_eq!(rewrite("   "), None);
    }

    #[test]
    fn cat_and_find_are_also_targets() {
        assert_eq!(
            rewrite("cat file.txt"),
            Some("sigfold cat file.txt".to_string())
        );
        assert_eq!(
            rewrite("find . -name x"),
            Some("sigfold find . -name x".to_string())
        );
    }
}
