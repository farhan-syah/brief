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
    /// `dirs::data_local_dir()/sigfold/folds`, or the `SIGFOLD_FOLD_DIR`
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
}
