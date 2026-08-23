//! Human-readable `sigfold report` output (`--format text`, the default).

use std::fmt::Write as _;

use super::aggregate::{ReportBody, ReportSummary};
use super::load::LoadResult;

/// Restated in every run's own output — see the scope-limits doc comment
/// in `report::mod` for the full reasoning. A report gets excerpted, and a
/// caveat that lives only in `--help` is lost when it does.
const SCOPE_CAVEAT: &str = "Scope: only grep, cat, find, and rg calls are tracked, so every number below is \
     \"output sigfold handled,\" never total output or your token usage.";
const LOWER_BOUND_CAVEAT: &str = "Lower bound: the re-read count below only catches re-reads that go back through \
     sigfold's own argv. A plain shell `cat` of a fold file is invisible to it.";

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
    let _ = writeln!(s, "{SCOPE_CAVEAT}");
    let _ = writeln!(s, "{LOWER_BOUND_CAVEAT}");
    let _ = writeln!(s);

    match body {
        ReportBody::NoData => {
            let _ = writeln!(
                s,
                "No tracking data yet — sigfold has not recorded any grep/cat/find/rg calls."
            );
        }
        ReportBody::AllMalformed => {
            let _ = writeln!(
                s,
                "{} line(s) were found but none parsed — the tracking file may be corrupted.",
                load.total_lines
            );
        }
        ReportBody::EmptyAfterFilter => {
            let project_note = if project { ", in this directory" } else { "" };
            let _ = writeln!(
                s,
                "No rows in {window_label}{project_note} — sigfold has recorded data, \
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
    let _ = writeln!(s, "sigfold report — window: {window_label}{project_note}");
    let _ = writeln!(s, "rows parsed: {}", load.parsed_total());
    let _ = writeln!(s, "malformed: {}", load.malformed);
}

fn render_data(s: &mut String, summary: &ReportSummary) {
    let _ = writeln!(s, "Handled totals ({} calls):", summary.row_count);
    let _ = writeln!(
        s,
        "  raw:  {} bytes (~{} tokens)",
        summary.raw_bytes, summary.raw_tokens
    );
    let _ = writeln!(
        s,
        "  kept: {} bytes (~{} tokens)",
        summary.kept_bytes, summary.kept_tokens
    );
    let _ = writeln!(
        s,
        "  {:.1}% of output sigfold handled was set aside to disk.",
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
                "  top 1%  ({} call(s)): {:.1}% of raw bytes",
                conc.top1.rows, conc.top1.pct_of_bytes
            );
            let _ = writeln!(
                s,
                "  top 5%  ({} call(s)): {:.1}% of raw bytes",
                conc.top5.rows, conc.top5.pct_of_bytes
            );
            let _ = writeln!(
                s,
                "  top 20% ({} call(s)): {:.1}% of raw bytes",
                conc.top20.rows, conc.top20.pct_of_bytes
            );
        }
    }
    let _ = writeln!(s);

    let _ = writeln!(s, "Re-read cost (lower bound):");
    let _ = writeln!(
        s,
        "  {} re-read(s) of {} folded call(s)",
        summary.reread.reread_rows, summary.reread.folded_rows
    );
    match summary.reread.rate() {
        None => {
            let _ = writeln!(s, "  no folds occurred, so there is no rate to report.");
        }
        Some(rate) => {
            let _ = writeln!(
                s,
                "  re-read rate: {rate:.1}% (lower bound; see scope note above)"
            );
            if summary.reread.reread_rows == 0 {
                let _ = writeln!(
                    s,
                    "  0 re-reads observed here proves nothing beyond this argv-visible \
                     signal — it is not evidence that no re-reading happened."
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
            p.program, p.calls, p.raw_bytes, p.kept_bytes, p.folded_count, p.reduction_pct
        );
    }
    let _ = writeln!(s);

    let _ = writeln!(
        s,
        "Non-zero exit: {} call(s) — context only, not a failure count (grep/rg \
         exit non-zero on \"no match\").",
        summary.nonzero_exit_count
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
    fn caveats_always_present() {
        let l = load(0, 0, vec![]);
        let text = render("all time", false, &l, &ReportBody::NoData);
        assert!(text.contains(SCOPE_CAVEAT));
        assert!(text.contains(LOWER_BOUND_CAVEAT));
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
        assert!(text.contains("malformed: 0"));
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
}
