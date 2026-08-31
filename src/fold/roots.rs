//! Per-path scoping: I/O side. Locates the roots file (or reads
//! `BRIEF_ROOTS`, which stands in for it entirely when set), canonicalizes
//! paths where possible, and decides whether the current directory is in
//! scope. `scope` holds the pure parsing/matching logic this wraps.

use std::io::Write;
use std::path::{Path, PathBuf};

use super::scope::{is_in_scope, parse_roots_env, parse_roots_file};

/// Whether the current directory is in scope for folding. `true` means
/// "fold as usual" — including when the roots file is absent, unreadable,
/// or yields no valid roots, which must not regress today's
/// fold-everywhere behavior.
pub(crate) fn cwd_in_scope(warn: &mut dyn Write) -> bool {
    cwd_in_scope_with(
        |key| std::env::var(key).ok(),
        dirs::config_dir(),
        std::env::current_dir().ok().as_deref(),
        warn,
    )
}

/// `cwd_in_scope` with every I/O input injected, so tests can drive
/// resolution, canonicalization, and the scope decision without touching
/// the real environment, config directory, or working directory.
pub(crate) fn cwd_in_scope_with(
    lookup: impl Fn(&str) -> Option<String>,
    config_dir: Option<PathBuf>,
    cwd: Option<&Path>,
    warn: &mut dyn Write,
) -> bool {
    let roots = resolve_roots(&lookup, config_dir, warn);

    // No cwd at all (e.g. `current_dir()` failed): never break the
    // command over this — fold as if scoping were not configured.
    let Some(cwd) = cwd else {
        return true;
    };

    let cwd = canonicalize_or_self(cwd);
    let roots: Vec<PathBuf> = roots.iter().map(|r| canonicalize_or_self(r)).collect();
    is_in_scope(&cwd, &roots)
}

/// `BRIEF_ROOTS`, when set, is the roots list directly (platform-separated)
/// and the file is never read — this is the escape hatch the roots file's
/// docs call out for tests and one-off runs. Otherwise read
/// `<config_dir>/brief/roots`; an absent or unreadable file yields no
/// roots (fold everywhere), same as a `BRIEF_ROOTS` entry never fatally
/// blocking the command.
fn resolve_roots(
    lookup: &impl Fn(&str) -> Option<String>,
    config_dir: Option<PathBuf>,
    warn: &mut dyn Write,
) -> Vec<PathBuf> {
    if let Some(val) = lookup("BRIEF_ROOTS") {
        return parse_roots_env(&val, warn);
    }
    let Some(dir) = config_dir else {
        return Vec::new();
    };
    let roots_path = dir.join("brief").join("roots");
    match std::fs::read_to_string(&roots_path) {
        Ok(contents) => parse_roots_file(&contents, warn),
        Err(_) => Vec::new(),
    }
}

/// Canonicalize `path`, falling back to it unchanged on any error — a
/// deleted or unreadable cwd, or a root that doesn't exist on disk, must
/// never break the command.
fn canonicalize_or_self(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_env(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn absent_roots_file_folds_everywhere() {
        let tmp = tempfile::tempdir().unwrap();
        let cwd = tmp.path().join("project");
        std::fs::create_dir_all(&cwd).unwrap();
        let config_dir = tmp.path().join("no-such-config-dir");

        let mut warn = Vec::new();
        let in_scope = cwd_in_scope_with(no_env, Some(config_dir), Some(&cwd), &mut warn);
        assert!(in_scope, "an absent roots file must fold everywhere");
    }

    #[test]
    fn roots_file_scopes_to_listed_root() {
        let tmp = tempfile::tempdir().unwrap();
        let config_dir = tmp.path().join("config");
        std::fs::create_dir_all(config_dir.join("brief")).unwrap();
        let root = tmp.path().join("allowed");
        std::fs::create_dir_all(&root).unwrap();
        let outside = tmp.path().join("elsewhere");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(
            config_dir.join("brief").join("roots"),
            format!("{}\n", root.display()),
        )
        .unwrap();

        let mut warn = Vec::new();
        assert!(cwd_in_scope_with(
            no_env,
            Some(config_dir.clone()),
            Some(&root),
            &mut warn
        ));

        let mut warn = Vec::new();
        assert!(!cwd_in_scope_with(
            no_env,
            Some(config_dir),
            Some(&outside),
            &mut warn
        ));
    }

    #[test]
    fn brief_roots_env_var_bypasses_the_file_entirely() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("allowed");
        std::fs::create_dir_all(&root).unwrap();
        // A roots file exists but must never be consulted once BRIEF_ROOTS
        // is set.
        let config_dir = tmp.path().join("config");
        std::fs::create_dir_all(config_dir.join("brief")).unwrap();
        std::fs::write(
            config_dir.join("brief").join("roots"),
            "/this/would/never/match\n",
        )
        .unwrap();

        let root_str = root.to_string_lossy().into_owned();
        let lookup = move |key: &str| (key == "BRIEF_ROOTS").then(|| root_str.clone());

        let mut warn = Vec::new();
        assert!(cwd_in_scope_with(
            lookup,
            Some(config_dir),
            Some(&root),
            &mut warn
        ));
    }

    #[test]
    fn all_invalid_roots_falls_back_to_fold_everywhere() {
        let tmp = tempfile::tempdir().unwrap();
        let config_dir = tmp.path().join("config");
        std::fs::create_dir_all(config_dir.join("brief")).unwrap();
        std::fs::write(
            config_dir.join("brief").join("roots"),
            "relative/one\nrelative/two\n",
        )
        .unwrap();

        let mut warn = Vec::new();
        let in_scope = cwd_in_scope_with(no_env, Some(config_dir), Some(tmp.path()), &mut warn);
        assert!(in_scope);
        assert!(!String::from_utf8(warn).unwrap().is_empty());
    }

    #[test]
    fn no_cwd_available_folds_everywhere_rather_than_breaking() {
        let mut warn = Vec::new();
        assert!(cwd_in_scope_with(no_env, None, None, &mut warn));
    }
}
