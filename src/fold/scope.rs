//! Pure per-path scoping logic: parsing roots-file/env-var text and
//! deciding whether a directory is in scope. No environment reads, no
//! filesystem access — see `fold::roots` for the I/O side (locating the
//! roots file, reading `BRIEF_ROOTS`, canonicalizing paths), matching how
//! `fold::config` and `track::paths` separate pure logic from I/O.

use std::io::Write;
use std::path::{Path, PathBuf};

/// Parse the roots file's contents: one absolute path per line, blank
/// lines ignored, a line whose first non-whitespace character is `#` is a
/// comment, trailing whitespace trimmed. A line that is not an absolute
/// path is skipped with a warning naming the line — never silently
/// dropped, never fatal.
pub(crate) fn parse_roots_file(contents: &str, warn: &mut dyn Write) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for (i, raw_line) in contents.lines().enumerate() {
        let line_no = i + 1;
        let trimmed = raw_line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some(root) = validate_absolute(trimmed) {
            roots.push(root);
        } else {
            let _ = writeln!(
                warn,
                "brief: roots file line {line_no}: {trimmed:?} is not an absolute path; skipping"
            );
        }
    }
    roots
}

/// Parse `BRIEF_ROOTS`: colon-separated absolute paths, for tests and
/// one-off runs. Stands in for the roots file entirely when set — same
/// per-entry validation, minus the file's comment/blank-line handling
/// (a single env var has no lines to skip).
pub(crate) fn parse_roots_env(val: &str, warn: &mut dyn Write) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for raw_entry in val.split(':') {
        let trimmed = raw_entry.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(root) = validate_absolute(trimmed) {
            roots.push(root);
        } else {
            let _ = writeln!(
                warn,
                "brief: BRIEF_ROOTS entry {trimmed:?} is not an absolute path; skipping"
            );
        }
    }
    roots
}

fn validate_absolute(candidate: &str) -> Option<PathBuf> {
    let path = Path::new(candidate);
    path.is_absolute().then(|| path.to_path_buf())
}

/// Whether `cwd` is at or under one of `roots`, component-wise — never a
/// string prefix, so `/home/a` never matches `/home/abc`. `Path::starts_with`
/// already compares whole components, not bytes. Empty `roots` means "fold
/// everywhere": the file/env yielded no valid roots, which must not
/// regress today's behavior.
pub(crate) fn is_in_scope(cwd: &Path, roots: &[PathBuf]) -> bool {
    roots.is_empty() || roots.iter().any(|root| cwd.starts_with(root))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_roots_file_skips_blanks_and_comments() {
        let mut warn = Vec::new();
        let roots = parse_roots_file(
            "\n  \n# a comment\n/home/user/proj\n   # indented comment\n/opt/work  \n",
            &mut warn,
        );
        assert_eq!(
            roots,
            vec![PathBuf::from("/home/user/proj"), PathBuf::from("/opt/work")]
        );
        assert!(warn.is_empty());
    }

    #[test]
    fn parse_roots_file_trims_trailing_whitespace() {
        let mut warn = Vec::new();
        let roots = parse_roots_file("/home/user/proj   \t\n", &mut warn);
        assert_eq!(roots, vec![PathBuf::from("/home/user/proj")]);
    }

    #[test]
    fn parse_roots_file_warns_and_skips_a_relative_path() {
        let mut warn = Vec::new();
        let roots = parse_roots_file("relative/path\n/home/user/proj\n", &mut warn);
        assert_eq!(roots, vec![PathBuf::from("/home/user/proj")]);
        let warning = String::from_utf8(warn).unwrap();
        assert!(warning.contains("line 1"));
        assert!(warning.contains("relative/path"));
    }

    #[test]
    fn parse_roots_file_all_invalid_yields_empty() {
        let mut warn = Vec::new();
        let roots = parse_roots_file("relative\nalso/relative\n", &mut warn);
        assert!(roots.is_empty());
    }

    #[test]
    fn parse_roots_env_splits_on_colon() {
        let mut warn = Vec::new();
        let roots = parse_roots_env("/home/a:/home/b", &mut warn);
        assert_eq!(
            roots,
            vec![PathBuf::from("/home/a"), PathBuf::from("/home/b")]
        );
        assert!(warn.is_empty());
    }

    #[test]
    fn parse_roots_env_skips_invalid_entries_with_a_warning() {
        let mut warn = Vec::new();
        let roots = parse_roots_env("/home/a:relative:/home/b", &mut warn);
        assert_eq!(
            roots,
            vec![PathBuf::from("/home/a"), PathBuf::from("/home/b")]
        );
        let warning = String::from_utf8(warn).unwrap();
        assert!(warning.contains("relative"));
    }

    #[test]
    fn is_in_scope_true_when_roots_empty() {
        assert!(is_in_scope(Path::new("/anywhere"), &[]));
    }

    #[test]
    fn is_in_scope_true_at_a_root_and_under_it() {
        let roots = vec![PathBuf::from("/home/a")];
        assert!(is_in_scope(Path::new("/home/a"), &roots));
        assert!(is_in_scope(Path::new("/home/a/sub/dir"), &roots));
    }

    #[test]
    fn is_in_scope_false_outside_every_root() {
        let roots = vec![PathBuf::from("/home/a")];
        assert!(!is_in_scope(Path::new("/home/b"), &roots));
    }

    #[test]
    fn is_in_scope_component_wise_not_string_prefix() {
        // A naive string prefix check would wrongly match "/home/abc"
        // against the root "/home/a".
        let roots = vec![PathBuf::from("/home/a")];
        assert!(!is_in_scope(Path::new("/home/abc"), &roots));
        assert!(!is_in_scope(Path::new("/home/abc/sub"), &roots));
    }
}
