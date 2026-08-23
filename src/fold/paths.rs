//! Fold-directory resolution, ported (with rtk coupling removed) plus
//! display/quoting helpers ported verbatim from rtk's `core::tee`.
//! Source: reference/rtk/src/core/tee.rs
//!
//! Deviations from rtk: no `Config::load()`, no `RTK_TEE_DIR` — the only
//! env override is `BRIEF_FOLD_DIR`, and the default directory is
//! `dirs::data_local_dir()/brief/folds` (rtk used `<data dir>/rtk/tee`).

use std::path::{Path, PathBuf};

use super::config::FoldConfig;

/// Resolve the fold directory: `BRIEF_FOLD_DIR` env var, then
/// `cfg.directory`, then the default `dirs::data_local_dir()/brief/folds`.
pub(crate) fn resolve_fold_dir(cfg: &FoldConfig) -> Option<PathBuf> {
    resolve_fold_dir_with(cfg, |key| std::env::var(key).ok())
}

/// `resolve_fold_dir` with an injected env lookup, so tests can drive the
/// precedence logic as a pure function instead of mutating the real process
/// environment (which is process-global state that would race other tests
/// running in parallel threads).
pub(crate) fn resolve_fold_dir_with(
    cfg: &FoldConfig,
    lookup: impl Fn(&str) -> Option<String>,
) -> Option<PathBuf> {
    if let Some(dir) = lookup("BRIEF_FOLD_DIR") {
        return Some(PathBuf::from(dir));
    }
    if let Some(ref dir) = cfg.directory {
        return Some(dir.clone());
    }
    dirs::data_local_dir().map(|d| d.join("brief").join("folds"))
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

/// Single recovery hint for a folded stream: states both that the full
/// output exists on disk and exactly how to reach the part that was cut
/// (the `tail` offset into the omitted middle). Collapsed from two
/// separate lines (rtk's `format_hint` + `format_tail_hint`) that used to
/// repeat the same long path — see `Fold::render`.
pub(crate) fn format_full_output_hint(path: &Path, line_offset: usize) -> String {
    format!(
        "[full output: tail -n +{} {}]",
        line_offset,
        display_shell_path(path)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_fold_dir_config_directory_used_when_env_unset() {
        let cfg = FoldConfig {
            directory: Some(PathBuf::from("/tmp/brief-cfg")),
            ..FoldConfig::default()
        };
        assert_eq!(
            resolve_fold_dir_with(&cfg, |_| None),
            Some(PathBuf::from("/tmp/brief-cfg")),
            "config directory used when env var is unset"
        );
    }

    #[test]
    fn resolve_fold_dir_env_wins_over_config_directory() {
        let cfg = FoldConfig {
            directory: Some(PathBuf::from("/tmp/brief-cfg")),
            ..FoldConfig::default()
        };
        let dir = resolve_fold_dir_with(&cfg, |key| {
            (key == "BRIEF_FOLD_DIR").then(|| "/tmp/brief-env-override".to_string())
        });
        assert_eq!(
            dir,
            Some(PathBuf::from("/tmp/brief-env-override")),
            "env var wins over config directory"
        );
    }

    #[test]
    fn resolve_fold_dir_default_ends_in_brief_folds() {
        let cfg = FoldConfig::default();
        let Some(dir) = resolve_fold_dir_with(&cfg, |_| None) else {
            return; // no data_local_dir on this platform/environment
        };
        assert!(dir.ends_with("brief/folds"));
    }

    #[test]
    fn format_full_output_hint_shape() {
        let path = PathBuf::from("/tmp/brief/folds/123_cargo_test.log");
        let hint = format_full_output_hint(&path, 51);
        assert!(hint.starts_with("[full output: tail -n +51 "));
        assert!(hint.ends_with(']'));
        assert!(hint.contains("123_cargo_test.log"));
    }

    #[test]
    fn display_shell_path_preserves_simple_paths() {
        let path = PathBuf::from("/tmp/brief/folds/123_cargo_test.log");
        assert_eq!(
            display_shell_path(&path),
            "/tmp/brief/folds/123_cargo_test.log"
        );
    }

    #[test]
    fn display_shell_path_quotes_paths_with_spaces() {
        let path = PathBuf::from("/tmp/brief/Application Support/123_go_test.log");
        assert_eq!(
            display_shell_path(&path),
            "\"/tmp/brief/Application Support/123_go_test.log\""
        );
    }

    #[test]
    fn display_shell_path_quotes_backslashes() {
        let path = PathBuf::from(r"/tmp/brief/folds/path\segment.log");
        assert_eq!(
            display_shell_path(&path),
            r#""/tmp/brief/folds/path\\segment.log""#
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
            .join("brief")
            .join("folds")
            .join("123_go_test.log");

        assert_eq!(
            display_shell_path(&path),
            "\"$HOME/Library/Application Support/brief/folds/123_go_test.log\""
        );
    }

    #[test]
    fn format_full_output_hint_avoids_backslash_escaped_whitespace() {
        let Some(home) = dirs::home_dir() else {
            return;
        };
        let path = home
            .join("Library")
            .join("Application Support")
            .join("brief")
            .join("folds")
            .join("123_go_test.log");
        let hint = format_full_output_hint(&path, 22);

        assert_eq!(
            hint,
            "[full output: tail -n +22 \"$HOME/Library/Application Support/brief/folds/123_go_test.log\"]"
        );
        assert!(
            !hint.contains("\\ "),
            "hint should not encourage backslash-escaped whitespace"
        );
    }
}
