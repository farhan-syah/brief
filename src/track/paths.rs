//! Tracking-file resolution, mirroring `fold::paths::resolve_fold_dir`'s
//! precedence: env var wins over the config field, which wins over the
//! default location.

use std::path::PathBuf;

use super::config::TrackConfig;

/// Resolve the tracking file path: `OGT_TRACK_FILE` env var, then
/// `cfg.path`, then the default `dirs::data_local_dir()/ogt/tracking.jsonl`.
pub(crate) fn resolve_track_path(cfg: &TrackConfig) -> Option<PathBuf> {
    resolve_track_path_with(cfg, |key| std::env::var(key).ok())
}

/// `resolve_track_path` with an injected env lookup, so tests can drive
/// the precedence logic as a pure function instead of mutating the real
/// process environment (which is process-global state that would race
/// other tests running in parallel threads).
pub(crate) fn resolve_track_path_with(
    cfg: &TrackConfig,
    lookup: impl Fn(&str) -> Option<String>,
) -> Option<PathBuf> {
    if let Some(path) = lookup("OGT_TRACK_FILE") {
        return Some(PathBuf::from(path));
    }
    if let Some(ref path) = cfg.path {
        return Some(path.clone());
    }
    dirs::data_local_dir().map(|d| d.join("ogt").join("tracking.jsonl"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_path_used_when_env_unset() {
        let cfg = TrackConfig {
            path: Some(PathBuf::from("/tmp/ogt-track-cfg.jsonl")),
            ..TrackConfig::default()
        };
        assert_eq!(
            resolve_track_path_with(&cfg, |_| None),
            Some(PathBuf::from("/tmp/ogt-track-cfg.jsonl")),
            "config path used when env var is unset"
        );
    }

    #[test]
    fn env_wins_over_config_path() {
        let cfg = TrackConfig {
            path: Some(PathBuf::from("/tmp/ogt-track-cfg.jsonl")),
            ..TrackConfig::default()
        };
        let path = resolve_track_path_with(&cfg, |key| {
            (key == "OGT_TRACK_FILE").then(|| "/tmp/ogt-track-env.jsonl".to_string())
        });
        assert_eq!(
            path,
            Some(PathBuf::from("/tmp/ogt-track-env.jsonl")),
            "env var wins over config path"
        );
    }

    #[test]
    fn default_ends_in_ogt_tracking_jsonl() {
        let cfg = TrackConfig::default();
        let Some(path) = resolve_track_path_with(&cfg, |_| None) else {
            return; // no data_local_dir on this platform/environment
        };
        assert!(path.ends_with("ogt/tracking.jsonl"));
    }
}
