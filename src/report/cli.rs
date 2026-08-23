//! `brief report`'s own argv parsing and top-level flow: resolve the
//! tracking file, load and filter rows, classify the outcome, and render.

use std::io::{self, Write};

use crate::track::{TrackConfig, now_ms, resolve_track_path};

use super::aggregate::classify;
use super::load::{LoadResult, load_rows};
use super::{render_json, render_text};

const USAGE: &str =
    "usage: brief report [--since <Nd|Nh|all|epoch_ms>] [--project] [--format text|json]\n";

/// Text for `brief report --help`. A function, not a `const`, because the
/// scope line names every program in `crate::targets::TARGETS`.
fn help_text() -> String {
    format!(
        "\
brief report — summarize the tracking JSONL brief has recorded.

Scope: only {} calls are tracked, so every number is
\"output brief handled,\" never total output or your token usage.

Lower bound: the re-read count only catches re-reads that go back through
brief's own argv. A plain shell cat of a fold file is invisible to it.

Flags:
  --since <Nd|Nh|all|epoch_ms>   window to report over (default: 30d)
  --project                      restrict to rows whose cwd is this directory
  --format text|json             output format (default: text)
  --help, -h                     this text

Usage: brief report [--since <spec>] [--project] [--format text|json]

To run a program literally named \"report\", invoke it by path:
brief ./report
",
        crate::targets::oxford_list()
    )
}

#[derive(Clone, Copy, PartialEq)]
enum Format {
    Text,
    Json,
}

/// Entry point wired from `cli::dispatch` for `brief report [...]`.
/// `args` is the argv following the literal `report` token.
pub(crate) fn run(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let track_cfg = TrackConfig::from_env();
    let cwd = std::env::current_dir()
        .ok()
        .and_then(|p| p.to_str().map(str::to_string));
    run_with(args, out, err, &track_cfg, now_ms(), cwd)
}

/// `run` with every ambient input injected, so tests can drive it as a
/// pure function instead of depending on the real process clock, cwd, or
/// tracking-file location.
pub(crate) fn run_with(
    args: &[String],
    out: &mut dyn Write,
    err: &mut dyn Write,
    track_cfg: &TrackConfig,
    now: u128,
    cwd: Option<String>,
) -> i32 {
    let mut since_spec = "30d".to_string();
    let mut project = false;
    let mut format = Format::Text;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--help" | "-h" => {
                let _ = out.write_all(help_text().as_bytes());
                return 0;
            }
            "--since" => {
                i += 1;
                match args.get(i) {
                    Some(v) => since_spec = v.clone(),
                    None => return usage_error(err),
                }
            }
            "--project" => project = true,
            "--format" => {
                i += 1;
                match args.get(i).map(String::as_str) {
                    Some("text") => format = Format::Text,
                    Some("json") => format = Format::Json,
                    _ => return usage_error(err),
                }
            }
            _ => return usage_error(err),
        }
        i += 1;
    }

    let since_ms = match parse_since(&since_spec, now) {
        Some(v) => v,
        None => return usage_error(err),
    };
    let window_label = describe_since(&since_spec);

    let empty_load = || LoadResult {
        rows: Vec::new(),
        malformed: 0,
        total_lines: 0,
    };

    let Some(path) = resolve_track_path(track_cfg) else {
        // No `dirs::data_local_dir()` on this platform and no override:
        // there is nowhere tracking could have written to, which is
        // observably the same as "no tracking data yet."
        return write_report(&window_label, project, &empty_load(), format, out);
    };

    let project_cwd = if project { cwd.as_deref() } else { None };

    let load = match load_rows(&path, since_ms, project_cwd) {
        Ok(l) => l,
        Err(e) if e.kind() == io::ErrorKind::NotFound => empty_load(),
        Err(e) => {
            let _ = writeln!(err, "brief report: could not read tracking file: {e}");
            return 1;
        }
    };

    write_report(&window_label, project, &load, format, out)
}

/// Classify, render (text or json per `format`), and write to `out`.
/// Always returns 0 — a report with zero matching rows is still a
/// successful run, not a usage error.
fn write_report(
    window_label: &str,
    project: bool,
    load: &LoadResult,
    format: Format,
    out: &mut dyn Write,
) -> i32 {
    let body = classify(load);
    let text = match format {
        Format::Text => render_text::render(window_label, project, load, &body),
        Format::Json => render_json::render(window_label, project, load, &body),
    };
    let _ = out.write_all(text.as_bytes());
    0
}

fn usage_error(err: &mut dyn Write) -> i32 {
    let _ = err.write_all(USAGE.as_bytes());
    2
}

const MS_PER_HOUR: u128 = 60 * 60 * 1000;
const MS_PER_DAY: u128 = 24 * MS_PER_HOUR;

/// Parse `--since`'s value into an absolute cutoff in ms-since-epoch.
/// `None` on the outer `Option` means "unparseable" (a usage error);
/// `Some(None)` means "all time" (no cutoff at all).
fn parse_since(spec: &str, now: u128) -> Option<Option<u128>> {
    if spec == "all" {
        return Some(None);
    }
    if let Some(rest) = spec.strip_suffix('d') {
        let n: u64 = rest.parse().ok()?;
        return Some(Some(now.saturating_sub(n as u128 * MS_PER_DAY)));
    }
    if let Some(rest) = spec.strip_suffix('h') {
        let n: u64 = rest.parse().ok()?;
        return Some(Some(now.saturating_sub(n as u128 * MS_PER_HOUR)));
    }
    let n: u128 = spec.parse().ok()?;
    Some(Some(n))
}

/// Relative-only window description — there is no date library in this
/// crate, so a raw epoch-ms `--since` (used for scripting/tests) is named
/// by its value rather than converted to a calendar date.
fn describe_since(spec: &str) -> String {
    if spec == "all" {
        return "all time".to_string();
    }
    if let Some(rest) = spec.strip_suffix('d')
        && let Ok(n) = rest.parse::<u64>()
    {
        return format!("last {n} day{}", if n == 1 { "" } else { "s" });
    }
    if let Some(rest) = spec.strip_suffix('h')
        && let Ok(n) = rest.parse::<u64>()
    {
        return format!("last {n} hour{}", if n == 1 { "" } else { "s" });
    }
    format!("since ts_ms {spec}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn cfg_with_path(path: &std::path::Path) -> TrackConfig {
        TrackConfig {
            path: Some(path.to_path_buf()),
            ..TrackConfig::default()
        }
    }

    /// A path that is guaranteed to never exist as a file: no test in this
    /// module (or any parallel run) ever writes to it, since the name is
    /// unique per call via a process-wide counter. `load_rows` opening it
    /// must return `NotFound`, never accidentally read another test's data.
    fn missing_path() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "brief-report-test-missing-{}-{n}.jsonl",
            std::process::id()
        ))
    }

    #[test]
    fn help_flag_prints_help_and_exits_0() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run_with(
            &["--help".to_string()],
            &mut out,
            &mut err,
            &cfg_with_path(&missing_path()),
            0,
            None,
        );
        assert_eq!(code, 0);
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("Scope:"));
        assert!(text.contains("Lower bound:"));
        assert!(err.is_empty());
    }

    #[test]
    fn short_help_flag_also_works() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run_with(
            &["-h".to_string()],
            &mut out,
            &mut err,
            &cfg_with_path(&missing_path()),
            0,
            None,
        );
        assert_eq!(code, 0);
        assert!(!out.is_empty());
        assert!(err.is_empty());
    }

    #[test]
    fn unknown_flag_is_a_usage_error() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run_with(
            &["--bogus".to_string()],
            &mut out,
            &mut err,
            &cfg_with_path(&missing_path()),
            0,
            None,
        );
        assert_eq!(code, 2);
        assert!(out.is_empty());
        assert!(!err.is_empty());
    }

    #[test]
    fn unparseable_since_is_a_usage_error() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run_with(
            &["--since".to_string(), "yesterday".to_string()],
            &mut out,
            &mut err,
            &cfg_with_path(&missing_path()),
            0,
            None,
        );
        assert_eq!(code, 2);
    }

    #[test]
    fn missing_flag_value_is_a_usage_error() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run_with(
            &["--since".to_string()],
            &mut out,
            &mut err,
            &cfg_with_path(&missing_path()),
            0,
            None,
        );
        assert_eq!(code, 2);
    }

    #[test]
    fn invalid_format_value_is_a_usage_error() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run_with(
            &["--format".to_string(), "xml".to_string()],
            &mut out,
            &mut err,
            &cfg_with_path(&missing_path()),
            0,
            None,
        );
        assert_eq!(code, 2);
    }

    #[test]
    fn missing_tracking_file_reports_no_data_not_an_error() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run_with(
            &[],
            &mut out,
            &mut err,
            &cfg_with_path(&missing_path()),
            0,
            None,
        );
        assert_eq!(code, 0);
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("No tracking data yet"));
        assert!(err.is_empty());
    }

    #[test]
    fn default_since_is_30_days() {
        assert_eq!(describe_since("30d"), "last 30 days");
    }

    #[test]
    fn since_all_means_no_cutoff() {
        assert_eq!(parse_since("all", 1_000_000), Some(None));
        assert_eq!(describe_since("all"), "all time");
    }

    #[test]
    fn since_days_computes_cutoff_from_now() {
        let now = 10 * MS_PER_DAY;
        assert_eq!(parse_since("3d", now), Some(Some(7 * MS_PER_DAY)));
    }

    #[test]
    fn since_hours_computes_cutoff_from_now() {
        let now = 10 * MS_PER_HOUR;
        assert_eq!(parse_since("4h", now), Some(Some(6 * MS_PER_HOUR)));
    }

    #[test]
    fn since_raw_epoch_ms_used_directly() {
        assert_eq!(parse_since("12345", 999_999_999), Some(Some(12345)));
    }

    #[test]
    fn since_garbage_is_unparseable() {
        assert_eq!(parse_since("banana", 0), None);
        assert_eq!(parse_since("3x", 0), None);
    }

    #[test]
    fn json_format_produces_json_not_text() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run_with(
            &["--format".to_string(), "json".to_string()],
            &mut out,
            &mut err,
            &cfg_with_path(&missing_path()),
            0,
            None,
        );
        assert_eq!(code, 0);
        let text = String::from_utf8(out).unwrap();
        assert!(
            text.trim_start().starts_with('{'),
            "json format must not print the text report"
        );
    }
}
