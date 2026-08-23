use std::io::{self, Write};
use std::path::PathBuf;

/// Default token threshold above which output is folded to disk.
const DEFAULT_THRESHOLD_TOKENS: usize = 25_000;

/// Default max rotated fold files kept per directory.
const DEFAULT_MAX_FILES: usize = 20;

/// Folding behavior: the size gate, rotation limit, and destination
/// directory. No rtk coupling — no `Config::load()`, no `RTK_*` env vars.
#[derive(Debug, Clone)]
pub struct FoldConfig {
    /// When false, `fold_output` always returns `Passthrough`.
    pub enabled: bool,
    /// Estimated-token threshold (see `tokens::estimate_tokens`) above
    /// which output is folded instead of passed through.
    pub threshold_tokens: usize,
    /// Max `*.log` files kept in the fold directory; oldest are rotated
    /// out first, the file just written always survives.
    pub max_files: usize,
    /// Fold directory override. `None` resolves to
    /// `dirs::data_local_dir()/brief/folds`, or the `BRIEF_FOLD_DIR`
    /// env var if set (env wins over this field).
    pub directory: Option<PathBuf>,
}

impl Default for FoldConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            threshold_tokens: DEFAULT_THRESHOLD_TOKENS,
            max_files: DEFAULT_MAX_FILES,
            directory: None,
        }
    }
}

impl FoldConfig {
    /// `Default`, layered with env var overrides: `BRIEF_THRESHOLD_TOKENS`
    /// (usize) and `BRIEF_ENABLED` (`0`/`false` disables). `directory`
    /// stays `None` — `paths::resolve_fold_dir` reads `BRIEF_FOLD_DIR`
    /// itself, at the point the fold directory is actually needed.
    pub fn from_env() -> Self {
        Self::from_env_with(&mut io::stderr(), |key| std::env::var(key).ok())
    }

    /// `from_env` with an explicit warning destination and an injected env
    /// lookup, so tests can drive the parsing/precedence logic as a pure
    /// function instead of mutating the real process environment (which is
    /// process-global state that would race other tests running in
    /// parallel threads).
    pub(crate) fn from_env_with(
        warn: &mut dyn Write,
        lookup: impl Fn(&str) -> Option<String>,
    ) -> Self {
        let mut cfg = Self::default();

        if let Some(val) = lookup("BRIEF_THRESHOLD_TOKENS") {
            match val.parse::<usize>() {
                Ok(n) => cfg.threshold_tokens = n,
                Err(_) => {
                    let _ = writeln!(
                        warn,
                        "brief: BRIEF_THRESHOLD_TOKENS={val:?} is not a valid number; \
                         using default ({DEFAULT_THRESHOLD_TOKENS})"
                    );
                }
            }
        }

        if let Some(val) = lookup("BRIEF_ENABLED") {
            match val.as_str() {
                "0" | "false" => cfg.enabled = false,
                "1" | "true" => cfg.enabled = true,
                _ => {
                    let _ = writeln!(
                        warn,
                        "brief: BRIEF_ENABLED={val:?} is not \
                         '0'/'false'/'1'/'true'; using default (enabled)"
                    );
                }
            }
        }

        cfg
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_values() {
        let cfg = FoldConfig::default();
        assert!(cfg.enabled);
        assert_eq!(cfg.threshold_tokens, 25_000);
        assert_eq!(cfg.max_files, 20);
        assert!(cfg.directory.is_none());
    }

    /// Builds a `lookup` closure from a fixed set of `(key, value)` pairs —
    /// stands in for the real process environment without touching it.
    fn env_map(pairs: &[(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        let pairs: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |key| pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
    }

    #[test]
    fn from_env_unset_uses_default() {
        let mut warn = Vec::new();
        let cfg = FoldConfig::from_env_with(&mut warn, env_map(&[]));
        assert_eq!(cfg.threshold_tokens, DEFAULT_THRESHOLD_TOKENS);
        assert!(cfg.enabled);
        assert!(warn.is_empty(), "unset vars must never warn");
    }

    #[test]
    fn from_env_valid_threshold_override_applies() {
        let mut warn = Vec::new();
        let cfg =
            FoldConfig::from_env_with(&mut warn, env_map(&[("BRIEF_THRESHOLD_TOKENS", "42")]));
        assert_eq!(cfg.threshold_tokens, 42);
        assert!(warn.is_empty());
    }

    #[test]
    fn from_env_invalid_threshold_falls_back_and_warns() {
        let mut warn = Vec::new();
        let cfg = FoldConfig::from_env_with(
            &mut warn,
            env_map(&[("BRIEF_THRESHOLD_TOKENS", "not-a-number")]),
        );
        assert_eq!(
            cfg.threshold_tokens, DEFAULT_THRESHOLD_TOKENS,
            "a bad env var must never break the command; it falls back"
        );
        let warning = String::from_utf8(warn).unwrap();
        assert!(warning.contains("BRIEF_THRESHOLD_TOKENS"));
        assert!(warning.contains("not-a-number"));
    }

    #[test]
    fn from_env_enabled_false_disables() {
        let mut warn = Vec::new();
        let cfg = FoldConfig::from_env_with(&mut warn, env_map(&[("BRIEF_ENABLED", "false")]));
        assert!(!cfg.enabled);
        assert!(warn.is_empty());
    }
}
