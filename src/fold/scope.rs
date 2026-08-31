//! Pure per-path scoping logic: parsing roots-file/env-var text and
//! deciding whether a directory is in scope. No environment reads, no
//! filesystem access — see `fold::roots` for the I/O side (locating the
//! roots file, reading `BRIEF_ROOTS`, canonicalizing paths), matching how
//! `fold::config` and `track::paths` separate pure logic from I/O.

use std::ffi::OsStr;
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

/// Parse `BRIEF_ROOTS`: platform-separated absolute paths, for tests and
/// one-off runs. Stands in for the roots file entirely when set — same
/// per-entry validation, minus the file's comment/blank-line handling
/// (a single env var has no lines to skip).
pub(crate) fn parse_roots_env(val: &str, warn: &mut dyn Write) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for raw_entry in std::env::split_paths(OsStr::new(val)) {
        let trimmed = raw_entry.to_string_lossy();
        let trimmed = trimmed.trim();
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
    use std::env;
    use std::path::{Path, PathBuf};

    use super::*;

    fn absolute_path(name: &str) -> PathBuf {
        env::temp_dir().join(name)
    }

    fn path_list(paths: &[&Path]) -> String {
        env::join_paths(paths.iter().copied())
            .unwrap()
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn parse_roots_file_skips_blanks_and_comments() {
        let project = absolute_path("brief-scope-project");
        let work = absolute_path("brief-scope-work");
        let contents = format!(
            "\n  \n# a comment\n{}\n   # indented comment\n{}  \n",
            project.display(),
            work.display()
        );
        let mut warn = Vec::new();
        let roots = parse_roots_file(&contents, &mut warn);
        assert_eq!(roots, vec![project, work]);
        assert!(warn.is_empty());
    }

    #[test]
    fn parse_roots_file_trims_trailing_whitespace() {
        let project = absolute_path("brief-scope-project");
        let mut warn = Vec::new();
        let roots = parse_roots_file(&format!("{}   \t\n", project.display()), &mut warn);
        assert_eq!(roots, vec![project]);
    }

    #[test]
    fn parse_roots_file_warns_and_skips_a_relative_path() {
        let project = absolute_path("brief-scope-project");
        let mut warn = Vec::new();
        let roots = parse_roots_file(
            &format!("relative/path\n{}\n", project.display()),
            &mut warn,
        );
        assert_eq!(roots, vec![project]);
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
    fn parse_roots_env_splits_on_the_platform_path_separator() {
        let first = absolute_path("brief-scope-a");
        let second = absolute_path("brief-scope-b");
        let value = path_list(&[&first, &second]);
        let mut warn = Vec::new();
        let roots = parse_roots_env(&value, &mut warn);
        assert_eq!(roots, vec![first, second]);
        assert!(warn.is_empty());
    }

    #[test]
    fn parse_roots_env_skips_invalid_entries_with_a_warning() {
        let first = absolute_path("brief-scope-a");
        let second = absolute_path("brief-scope-b");
        let relative = Path::new("relative");
        let value = path_list(&[&first, relative, &second]);
        let mut warn = Vec::new();
        let roots = parse_roots_env(&value, &mut warn);
        assert_eq!(roots, vec![first, second]);
        let warning = String::from_utf8(warn).unwrap();
        assert!(warning.contains("relative"));
    }

    #[test]
    fn is_in_scope_true_when_roots_empty() {
        assert!(is_in_scope(Path::new("/anywhere"), &[]));
    }

    #[test]
    fn is_in_scope_true_at_a_root_and_under_it() {
        let root = absolute_path("brief-scope-root");
        let roots = vec![root.clone()];
        assert!(is_in_scope(&root, &roots));
        assert!(is_in_scope(&root.join("sub").join("dir"), &roots));
    }

    #[test]
    fn is_in_scope_false_outside_every_root() {
        let roots = vec![absolute_path("brief-scope-root")];
        let outside = absolute_path("brief-scope-outside");
        assert!(!is_in_scope(&outside, &roots));
    }

    #[test]
    fn is_in_scope_component_wise_not_string_prefix() {
        let root = absolute_path("brief-scope-root");
        let sibling = absolute_path("brief-scope-root-other");
        let roots = vec![root];
        assert!(!is_in_scope(&sibling, &roots));
        assert!(!is_in_scope(&sibling.join("sub"), &roots));
    }
}
