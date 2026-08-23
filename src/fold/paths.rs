//! Fold-directory resolution, ported (with rtk coupling removed) plus
//! display/quoting helpers ported verbatim from rtk's `core::tee`.
//! Source: reference/rtk/src/core/tee.rs
//!
//! Deviations from rtk: no `Config::load()`, no `RTK_TEE_DIR` — the only
//! env override is `SIGFOLD_FOLD_DIR`, and the default directory is
//! `dirs::data_local_dir()/sigfold/folds` (rtk used `<data dir>/rtk/tee`).

use std::path::{Path, PathBuf};

use super::config::FoldConfig;

/// Resolve the fold directory: `SIGFOLD_FOLD_DIR` env var, then
/// `cfg.directory`, then the default `dirs::data_local_dir()/sigfold/folds`.
pub(crate) fn resolve_fold_dir(cfg: &FoldConfig) -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("SIGFOLD_FOLD_DIR") {
        return Some(PathBuf::from(dir));
    }
    if let Some(ref dir) = cfg.directory {
        return Some(dir.clone());
    }
    dirs::data_local_dir().map(|d| d.join("sigfold").join("folds"))
}

/// Ported verbatim from rtk's `core::tee::display_path`.
pub(crate) fn display_path(path: &Path) -> String {
    if let Some(home) = dirs::home_dir()
        && let Ok(relative) = path.strip_prefix(&home)
    {
        return format!("~/{}", relative.display());
    }
    path.display().to_string()
}

/// Ported verbatim from rtk's `core::tee::needs_shell_quoting`.
fn needs_shell_quoting(path: &str) -> bool {
    path.chars().any(|c| {
        c.is_whitespace()
            || matches!(
                c,
                '\'' | '"'
                    | '\\'
                    | '$'
                    | '`'
                    | '!'
                    | '#'
                    | '&'
                    | '('
                    | ')'
                    | ';'
                    | '<'
                    | '>'
                    | '?'
                    | '['
                    | ']'
                    | '{'
                    | '}'
                    | '|'
                    | '*'
            )
    })
}

/// Ported verbatim from rtk's `core::tee::escape_double_quoted_path`.
fn escape_double_quoted_path(path: &str) -> String {
    let mut escaped = String::with_capacity(path.len());
    for c in path.chars() {
        if matches!(c, '\\' | '"' | '$' | '`') {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    escaped
}

/// Ported verbatim from rtk's `core::tee::display_shell_path`.
pub(crate) fn display_shell_path(path: &Path) -> String {
    let display = display_path(path);
    if !needs_shell_quoting(&display) {
        return display;
    }

    if let Some(relative) = display.strip_prefix("~/") {
        let relative = relative.replace(std::path::MAIN_SEPARATOR, "/");
        return format!("\"$HOME/{}\"", escape_double_quoted_path(&relative));
    }

    format!("\"{}\"", escape_double_quoted_path(&display))
}

/// Ported (renamed) from rtk's `core::tee::format_hint`.
pub(crate) fn format_hint(path: &Path) -> String {
    format!("[full output: {}]", display_shell_path(path))
}

/// Ported (renamed) from rtk's `core::tee::force_tee_tail_hint`'s hint
/// format string.
pub(crate) fn format_tail_hint(path: &Path, line_offset: usize) -> String {
    format!(
        "[see remaining: tail -n +{} {}]",
        line_offset,
        display_shell_path(path)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // Both env-touching cases live in one test: SIGFOLD_FOLD_DIR is
    // process-global, and cargo runs tests in parallel threads by default,
    // so a separate set/remove pair here could race another test's read.
    #[test]
    fn resolve_fold_dir_env_and_config_precedence() {
        let cfg = FoldConfig {
            directory: Some(PathBuf::from("/tmp/sigfold-cfg")),
            ..FoldConfig::default()
        };
        assert_eq!(
            resolve_fold_dir(&cfg),
            Some(PathBuf::from("/tmp/sigfold-cfg")),
            "config directory used when env var is unset"
        );

        // SAFETY: env var is unique to this test within the crate.
        unsafe {
            std::env::set_var("SIGFOLD_FOLD_DIR", "/tmp/sigfold-env-override");
        }
        let dir = resolve_fold_dir(&cfg);
        unsafe {
            std::env::remove_var("SIGFOLD_FOLD_DIR");
        }
        assert_eq!(
            dir,
            Some(PathBuf::from("/tmp/sigfold-env-override")),
            "env var wins over config directory"
        );
    }

    #[test]
    fn resolve_fold_dir_default_ends_in_sigfold_folds() {
        let cfg = FoldConfig::default();
        let Some(dir) = resolve_fold_dir(&cfg) else {
            return; // no data_local_dir on this platform/environment
        };
        assert!(dir.ends_with("sigfold/folds"));
    }

    #[test]
    fn format_hint_shape() {
        let path = PathBuf::from("/tmp/sigfold/folds/123_cargo_test.log");
        let hint = format_hint(&path);
        assert!(hint.starts_with("[full output: "));
        assert!(hint.ends_with(']'));
        assert!(hint.contains("123_cargo_test.log"));
    }

    #[test]
    fn display_shell_path_preserves_simple_paths() {
        let path = PathBuf::from("/tmp/sigfold/folds/123_cargo_test.log");
        assert_eq!(
            display_shell_path(&path),
            "/tmp/sigfold/folds/123_cargo_test.log"
        );
    }

    #[test]
    fn display_shell_path_quotes_paths_with_spaces() {
        let path = PathBuf::from("/tmp/sigfold/Application Support/123_go_test.log");
        assert_eq!(
            display_shell_path(&path),
            "\"/tmp/sigfold/Application Support/123_go_test.log\""
        );
    }

    #[test]
    fn display_shell_path_quotes_backslashes() {
        let path = PathBuf::from(r"/tmp/sigfold/folds/path\segment.log");
        assert_eq!(
            display_shell_path(&path),
            r#""/tmp/sigfold/folds/path\\segment.log""#
        );
    }

    #[test]
    fn display_shell_path_uses_home_var_for_home_paths_with_spaces() {
        let Some(home) = dirs::home_dir() else {
            return;
        };
        let path = home
            .join("Library")
            .join("Application Support")
            .join("sigfold")
            .join("folds")
            .join("123_go_test.log");

        assert_eq!(
            display_shell_path(&path),
            "\"$HOME/Library/Application Support/sigfold/folds/123_go_test.log\""
        );
    }

    #[test]
    fn format_hint_avoids_backslash_escaped_whitespace() {
        let Some(home) = dirs::home_dir() else {
            return;
        };
        let path = home
            .join("Library")
            .join("Application Support")
            .join("sigfold")
            .join("folds")
            .join("123_go_test.log");
        let hint = format_hint(&path);

        assert_eq!(
            hint,
            "[full output: \"$HOME/Library/Application Support/sigfold/folds/123_go_test.log\"]"
        );
        assert!(
            !hint.contains("\\ "),
            "hint should not encourage backslash-escaped whitespace"
        );
    }

    #[test]
    fn format_tail_hint_shape() {
        let path = PathBuf::from("/tmp/sigfold/folds/123_docker_images.log");
        let hint = format_tail_hint(&path, 22);
        assert!(hint.starts_with("[see remaining: tail -n +22 "));
        assert!(hint.ends_with(']'));
        assert!(hint.contains("123_docker_images.log"));
    }
}
