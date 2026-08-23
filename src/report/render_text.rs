//! Human-readable `brief report` output (`--format text`, the default).

use std::fmt::Write as _;

use super::aggregate::{ReportBody, ReportSummary};
// Short forms: each is attached directly to the number it qualifies rather
// than printed as a detached block, so it survives excerpting of that line.
use super::caveats::{LOWER_BOUND_SHORT as LOWER_BOUND_CAVEAT, SCOPE_SHORT as SCOPE_NOTE};
use super::load::LoadResult;
use crate::thousands::with_thousands_separator as sep;

/// `n` and the correctly pluralized `noun` (`1 call`, `2 calls`).
fn plural(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("{n} {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

/// Render the full text report for `window_label` (e.g. "last 30 days",
/// "all time"), `project` (whether `--project` was passed), `load`
/// (header counts), and `body` (the classified outcome).
pub(crate) fn render(
    window_label: &str,
    project: bool,
    load: &LoadResult,
    body: &ReportBody,
) -> String {
    let mut s = String::new();
    render_header(&mut s, window_label, project, load);
    let _ = writeln!(s);

    match body {
        ReportBody::NoData => {
            let _ = writeln!(
                s,
                "No tracking data yet — brief has not recorded any grep/cat/find/rg calls."
            );
        }
        ReportBody::AllMalformed => {
            let (verb, line_word) = if load.total_lines == 1 {
                ("was", "line")
            } else {
                ("were", "lines")
            };
            let _ = writeln!(
                s,
                "{} {line_word} {verb} found but none parsed — the tracking file may be corrupted.",
                load.total_lines
            );
        }
        ReportBody::EmptyAfterFilter => {
            let project_note = if project { ", in this directory" } else { "" };
            let _ = writeln!(
                s,
                "No rows in {window_label}{project_note} — brief has recorded data, \
                 just none in this window."
            );
        }
        ReportBody::Data(summary) => render_data(&mut s, summary),
    }

    s
}

fn render_header(s: &mut String, window_label: &str, project: bool, load: &LoadResult) {
    let project_note = if project {
        " (this directory only)"
    } else {
        ""
    };
    // Report the in-window count against the file total when they differ.
    // Showing only the file total next to a summary computed from the
    // filtered rows leaves the reader unable to explain the gap between
    // the two numbers, which reads as a bug in the report.
    let in_window = load.rows.len();
    let parsed = load.parsed_total();
    let counted = if in_window == parsed {
        plural(parsed, "row")
    } else {
        format!("{in_window} of {}", plural(parsed, "row"))
    };
    let _ = writeln!(
        s,
        "brief report — {window_label}{project_note} · {counted} · {} malformed",
        load.malformed
    );
}

fn render_data(s: &mut String, summary: &ReportSummary) {
    let _ = writeln!(
        s,
        "Handled totals ({} — {SCOPE_NOTE}):",
        plural(summary.row_count, "call")
    );
    let _ = writeln!(
        s,
        "  raw:  {} bytes (~{} tokens)",
        sep(summary.raw_bytes as usize),
        sep(summary.raw_tokens)
    );
    let _ = writeln!(
        s,
        "  kept: {} bytes (~{} tokens)",
        sep(summary.kept_bytes as usize),
        sep(summary.kept_tokens)
    );
    let _ = writeln!(
        s,
        "  {:.1}% of output brief handled was set aside to disk.",
        summary.set_aside_pct
    );
    let _ = writeln!(s);

    let _ = writeln!(
        s,
        "Concentration (by raw bytes, calls sorted largest first):"
    );
    match &summary.concentration {
        None => {
            let _ = writeln!(
                s,
                "  fewer than 20 calls in this window — too small a sample for a \
                 percentile breakdown to mean anything."
            );
        }
        Some(conc) => {
            let _ = writeln!(
                s,
                "  top 1%  ({}): {:.1}% of raw bytes",
                plural(conc.top1.rows, "call"),
                conc.top1.pct_of_bytes
            );
            let _ = writeln!(
                s,
                "  top 5%  ({}): {:.1}% of raw bytes",
                plural(conc.top5.rows, "call"),
                conc.top5.pct_of_bytes
            );
            let _ = writeln!(
                s,
                "  top 20% ({}): {:.1}% of raw bytes",
                plural(conc.top20.rows, "call"),
                conc.top20.pct_of_bytes
            );
        }
    }
    let _ = writeln!(s);

    let _ = writeln!(s, "Re-read cost ({LOWER_BOUND_CAVEAT}):");
    let _ = writeln!(
        s,
        "  {} of {}",
        plural(summary.reread.reread_rows, "re-read"),
        plural(summary.reread.folded_rows, "folded call")
    );
    match summary.reread.rate() {
        None => {
            let _ = writeln!(s, "  no folds occurred, so there is no rate to report.");
        }
        Some(rate) => {
            let _ = writeln!(s, "  re-read rate: {rate:.1}%");
            if summary.reread.reread_rows == 0 {
                let _ = writeln!(
                    s,
                    "  0 observed here — argv-visible signal only, not proof none happened."
                );
            }
        }
    }
    let _ = writeln!(s);

    let _ = writeln!(s, "Per-program (sorted by raw bytes, largest first):");
    let _ = writeln!(
        s,
        "  {:<8} {:>8} {:>14} {:>14} {:>8} {:>10}",
        "program", "calls", "raw bytes", "kept bytes", "folded", "reduction"
    );
    for p in &summary.programs {
        let _ = writeln!(
            s,
            "  {:<8} {:>8} {:>14} {:>14} {:>8} {:>9.1}%",
            p.program,
            p.calls,
            sep(p.raw_bytes as usize),
            sep(p.kept_bytes as usize),
            p.folded_count,
            p.reduction_pct
        );
    }
    let _ = writeln!(s);

    let _ = writeln!(
        s,
        "Non-zero exit: {} — context only, not a failure count (grep/rg \
         exit non-zero on \"no match\").",
        plural(summary.nonzero_exit_count, "call")
    );
}

#[cfg(test)]
mod tests {
    use super::super::aggregate::aggregate;
    use super::super::parse::ReportRow;
    use super::*;

    fn load(total_lines: usize, malformed: usize, rows: Vec<ReportRow>) -> LoadResult {
        LoadResult {
            rows,
            malformed,
            total_lines,
        }
    }

    fn row(program: &str, raw: u64, kept: u64) -> ReportRow {
        ReportRow {
            ts_ms: 0,
            program: program.to_string(),
            cwd: None,
            exit_code: 0,
            stdout_raw_bytes: raw,
            stdout_kept_bytes: kept,
            stdout_folded: false,
            stderr_raw_bytes: 0,
            stderr_kept_bytes: 0,
            stderr_folded: false,
            reads_fold: false,
        }
    }

    #[test]
    fn caveats_attached_to_the_numbers_they_qualify() {
        let rows = vec![row("grep", 10, 10)];
        let l = load(1, 0, rows.clone());
        let text = render("all time", false, &l, &ReportBody::Data(aggregate(&rows)));
        assert!(text.contains(SCOPE_NOTE));
        assert!(text.contains(LOWER_BOUND_CAVEAT));
        assert!(
            text.contains(&format!("Handled totals (1 call — {SCOPE_NOTE}):")),
            "scope note must be attached to the Handled totals line: {text}"
        );
    }

    #[test]
    fn header_shows_in_window_count_against_file_total_when_filtered() {
        // 5 rows parsed from the file, 1 survived the window filter.
        let rows = vec![row("grep", 10, 10)];
        let l = LoadResult {
            rows,
            malformed: 0,
            total_lines: 5,
        };
        let text = render(
            "last 1 hour",
            false,
            &l,
            &ReportBody::Data(aggregate(&l.rows)),
        );
        assert!(
            text.contains("1 of 5 rows"),
            "a filtered report must explain the gap between file total and \
             the count its numbers are computed from, got: {text}"
        );
    }

    #[test]
    fn header_always_prints_malformed_count_even_when_zero() {
        let l = load(3, 0, vec![row("grep", 10, 10)]);
        let text = render(
            "last 30 days",
            false,
            &l,
            &ReportBody::Data(aggregate(&l.rows)),
        );
        assert!(text.contains("0 malformed"));
    }

    #[test]
    fn no_data_message_distinct_from_empty_after_filter_message() {
        let empty = load(0, 0, vec![]);
        let no_data_text = render("last 7 days", false, &empty, &ReportBody::NoData);
        assert!(no_data_text.contains("No tracking data yet"));

        let has_data = load(5, 0, vec![]);
        let filtered_text = render(
            "last 7 days",
            false,
            &has_data,
            &ReportBody::EmptyAfterFilter,
        );
        assert!(filtered_text.contains("last 7 days"));
        assert!(filtered_text.contains("just none in this window"));
        assert_ne!(no_data_text, filtered_text);
    }

    #[test]
    fn reread_rate_none_prints_no_rate_line_not_zero_percent() {
        let rows = vec![row("grep", 10, 10)]; // no folds at all
        let l = load(1, 0, rows.clone());
        let text = render("all time", false, &l, &ReportBody::Data(aggregate(&rows)));
        assert!(text.contains("no folds occurred"));
        // Scoped to the re-read section on purpose: 0.0% is legitimate
        // elsewhere in this report (raw == kept means 0.0% set aside, and the
        // per-program reduction is 0.0% too). Asserting over the whole text
        // would fail on correct output.
        assert!(!text.contains("re-read rate:"));
    }

    #[test]
    fn small_sample_prints_sample_too_small_not_percentiles() {
        let rows = vec![row("grep", 10, 10)];
        let l = load(1, 0, rows.clone());
        let text = render("all time", false, &l, &ReportBody::Data(aggregate(&rows)));
        assert!(text.contains("too small a sample"));
        assert!(!text.contains("top 1%"));
    }

    #[test]
    fn plural_is_singular_for_one_and_plural_otherwise() {
        assert_eq!(plural(0, "call"), "0 calls");
        assert_eq!(plural(1, "call"), "1 call");
        assert_eq!(plural(2, "call"), "2 calls");
    }

    #[test]
    fn handled_totals_uses_singular_call_for_one_row() {
        let rows = vec![row("grep", 10, 10)];
        let l = load(1, 0, rows.clone());
        let text = render("all time", false, &l, &ReportBody::Data(aggregate(&rows)));
        assert!(
            text.contains("Handled totals (1 call —"),
            "must say '1 call', not '1 calls': {text}"
        );
    }
}
