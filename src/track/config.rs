use std::io::{self, Write};
use std::path::PathBuf;

/// Retention window, in days. Compaction drops a row once EITHER this
/// window or the byte budget below says to drop it — the byte budget is
/// what actually bounds the file's size; this window only ever removes
/// more rows than the budget alone would.
const DEFAULT_RETENTION_DAYS: u64 = 90;

/// Size compaction reduces the tracking file to.
const DEFAULT_COMPACT_TARGET_BYTES: u64 = 32 * 1024 * 1024;

/// Size that triggers compaction. The 8 MiB gap to
/// `DEFAULT_COMPACT_TARGET_BYTES` is what makes compaction amortized O(1):
/// each compaction reclaims at least that much, so it can fire at most once
/// per 8 MiB appended. This also hard-bounds the file at 40 MiB no matter
/// how heavily the tool is used.
const DEFAULT_COMPACT_TRIGGER_BYTES: u64 = 40 * 1024 * 1024;

/// Tracking behavior: whether it runs at all, the destination file, how
/// long rows are kept, and the compaction thresholds. Mirrors
/// `fold::config::FoldConfig`'s shape. The compaction fields are plain
/// (non-env-configurable) fields rather than constants so tests can inject
/// small thresholds and stay fast and hermetic, the same reason
/// `from_env_with` injects its env lookup instead of reading the process
/// environment directly.
#[derive(Debug, Clone)]
pub struct TrackConfig {
    /// When false, `track::append` is always a no-op.
    pub enabled: bool,
    /// Tracking file override. `None` resolves to
    /// `dirs::data_local_dir()/ogt/tracking.jsonl`, or the
    /// `OGT_TRACK_FILE` env var if set (env wins over this field).
    pub path: Option<PathBuf>,
    /// Rows older than this many days are dropped on compaction.
    pub retention_days: u64,
    /// Compaction fires once the tracking file reaches this size.
    pub compact_trigger_bytes: u64,
    /// Compaction rewrites the file down to at most this size, keeping the
    /// newest rows.
    pub compact_target_bytes: u64,
}

impl Default for TrackConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            path: None,
            retention_days: DEFAULT_RETENTION_DAYS,
            compact_trigger_bytes: DEFAULT_COMPACT_TRIGGER_BYTES,
            compact_target_bytes: DEFAULT_COMPACT_TARGET_BYTES,
        }
    }
}

impl TrackConfig {
    /// `Default`, layered with an env var override: `OGT_TRACK_ENABLED`
    /// (`0`/`false` disables). `path` stays `None` —
    /// `paths::resolve_track_path` reads `OGT_TRACK_FILE` itself, at
    /// the point the tracking file is actually needed.
    pub fn from_env() -> Self {
        Self::from_env_with(&mut io::stderr(), |key| std::env::var(key).ok())
    }

    /// `from_env` with an explicit warning destination and an injected env
    /// lookup, so tests can drive the parsing logic as a pure function
    /// instead of mutating the real process environment (which is
    /// process-global state that would race other tests running in
    /// parallel threads).
    pub(crate) fn from_env_with(
        warn: &mut dyn Write,
        lookup: impl Fn(&str) -> Option<String>,
    ) -> Self {
        let mut cfg = Self::default();

        if let Some(val) = lookup("OGT_TRACK_ENABLED") {
            match val.as_str() {
                "0" | "false" => cfg.enabled = false,
                "1" | "true" => cfg.enabled = true,
                _ => {
                    let _ = writeln!(
                        warn,
                        "ogt: OGT_TRACK_ENABLED={val:?} is not \
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
        let cfg = TrackConfig::default();
        assert!(cfg.enabled);
        assert_eq!(cfg.retention_days, 90);
        assert!(cfg.path.is_none());
        assert_eq!(cfg.compact_trigger_bytes, 40 * 1024 * 1024);
        assert_eq!(cfg.compact_target_bytes, 32 * 1024 * 1024);
        assert!(
            cfg.compact_target_bytes < cfg.compact_trigger_bytes,
            "the gap between target and trigger is what keeps compaction amortized O(1)"
        );
    }

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
        let cfg = TrackConfig::from_env_with(&mut warn, env_map(&[]));
        assert!(cfg.enabled);
        assert!(warn.is_empty(), "unset vars must never warn");
    }

    #[test]
    fn from_env_disabled_false_disables() {
        let mut warn = Vec::new();
        let cfg = TrackConfig::from_env_with(&mut warn, env_map(&[("OGT_TRACK_ENABLED", "false")]));
        assert!(!cfg.enabled);
        assert!(warn.is_empty());
    }

    #[test]
    fn from_env_invalid_value_falls_back_and_warns() {
        let mut warn = Vec::new();
        let cfg =
            TrackConfig::from_env_with(&mut warn, env_map(&[("OGT_TRACK_ENABLED", "not-a-bool")]));
        assert!(
            cfg.enabled,
            "a bad env var must never break the command; it falls back"
        );
        let warning = String::from_utf8(warn).unwrap();
        assert!(warning.contains("OGT_TRACK_ENABLED"));
        assert!(warning.contains("not-a-bool"));
    }
}
