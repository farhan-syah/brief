//! Machine-readable `brief report` output (`--format json`). Hand-rolled,
//! flat-ish JSON — no `serde` — reusing `track::json_string` for escaping
//! rather than writing a second escaper that could drift from `parse.rs`'s
//! unescaper.

use std::fmt::Write as _;

use crate::track::json_string;

use super::aggregate::{ReportBody, ReportSummary};
// Full forms: a JSON consumer may show these with no adjacent number to
// lend them context, so they state the whole fact.
use super::caveats::{LOWER_BOUND_FULL as LOWER_BOUND_CAVEAT, SCOPE_FULL as SCOPE_CAVEAT};
use super::load::LoadResult;

pub(crate) fn render(
    window_label: &str,
    project: bool,
    load: &LoadResult,
    body: &ReportBody,
) -> String {
    let mut s = String::new();
    let _ = write!(s, "{{");
    let _ = write!(s, "\"window\":{}", json_string(window_label));
    let _ = write!(s, ",\"project\":{project}");
    let _ = write!(s, ",\"rows_parsed\":{}", load.parsed_total());
    let _ = write!(s, ",\"malformed\":{}", load.malformed);
    let _ = write!(
        s,
        ",\"caveats\":{{\"scope\":{},\"lower_bound\":{}}}",
        json_string(SCOPE_CAVEAT),
        json_string(LOWER_BOUND_CAVEAT)
    );

    match body {
        ReportBody::NoData => {
            let _ = write!(s, ",\"status\":\"no_data\"");
        }
        ReportBody::AllMalformed => {
            let _ = write!(s, ",\"status\":\"all_malformed\"");
        }
        ReportBody::EmptyAfterFilter => {
            let _ = write!(s, ",\"status\":\"empty_after_filter\"");
        }
        ReportBody::Data(summary) => {
            let _ = write!(s, ",\"status\":\"ok\"");
            render_data(&mut s, summary);
        }
    }

    let _ = writeln!(s, "}}");
    s
}

fn render_data(s: &mut String, summary: &ReportSummary) {
    let _ = write!(s, ",\"handled\":{{");
    let _ = write!(s, "\"calls\":{}", summary.row_count);
    let _ = write!(s, ",\"raw_bytes\":{}", summary.raw_bytes);
    let _ = write!(s, ",\"kept_bytes\":{}", summary.kept_bytes);
    let _ = write!(s, ",\"raw_tokens\":{}", summary.raw_tokens);
    let _ = write!(s, ",\"kept_tokens\":{}", summary.kept_tokens);
    let _ = write!(s, ",\"set_aside_pct\":{:.4}", summary.set_aside_pct);
    let _ = write!(s, "}}");

    let _ = write!(s, ",\"concentration\":");
    match &summary.concentration {
        None => {
            let _ = write!(s, "null");
        }
        Some(conc) => {
            let _ = write!(s, "{{");
            let bands = [
                ("top1", &conc.top1),
                ("top5", &conc.top5),
                ("top20", &conc.top20),
            ];
            for (i, (name, band)) in bands.iter().enumerate() {
                if i > 0 {
                    let _ = write!(s, ",");
                }
                let _ = write!(
                    s,
                    "\"{name}\":{{\"rows\":{},\"pct_of_bytes\":{:.4}}}",
                    band.rows, band.pct_of_bytes
                );
            }
            let _ = write!(s, "}}");
        }
    }

    let _ = write!(s, ",\"reread\":{{");
    let _ = write!(s, "\"reread_rows\":{}", summary.reread.reread_rows);
    let _ = write!(s, ",\"folded_rows\":{}", summary.reread.folded_rows);
    let _ = write!(s, ",\"rate_pct\":");
    match summary.reread.rate() {
        None => {
            let _ = write!(s, "null");
        }
        Some(rate) => {
            let _ = write!(s, "{rate:.4}");
        }
    }
    let _ = write!(s, "}}");

    let _ = write!(s, ",\"programs\":[");
    for (i, p) in summary.programs.iter().enumerate() {
        if i > 0 {
            let _ = write!(s, ",");
        }
        let _ = write!(
            s,
            "{{\"program\":{},\"calls\":{},\"raw_bytes\":{},\"kept_bytes\":{},\"folded_count\":{},\"reduction_pct\":{:.4}}}",
            json_string(&p.program),
            p.calls,
            p.raw_bytes,
            p.kept_bytes,
            p.folded_count,
            p.reduction_pct
        );
    }
    let _ = write!(s, "]");

    let _ = write!(s, ",\"nonzero_exit_count\":{}", summary.nonzero_exit_count);
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
    fn no_data_status() {
        let l = load(0, 0, vec![]);
        let text = render("all time", false, &l, &ReportBody::NoData);
        assert!(text.contains("\"status\":\"no_data\""));
        assert!(text.contains("\"malformed\":0"));
    }

    #[test]
    fn data_status_includes_handled_totals_and_programs() {
        let rows = vec![row("grep", 1000, 200)];
        let l = load(1, 0, rows.clone());
        let text = render(
            "last 30 days",
            true,
            &l,
            &ReportBody::Data(aggregate(&rows)),
        );
        assert!(text.contains("\"status\":\"ok\""));
        assert!(text.contains("\"raw_bytes\":1000"));
        assert!(text.contains("\"kept_bytes\":200"));
        assert!(text.contains("\"program\":\"grep\""));
        assert!(text.contains("\"project\":true"));
    }

    #[test]
    fn reread_rate_null_when_no_folds() {
        let rows = vec![row("grep", 10, 10)];
        let l = load(1, 0, rows.clone());
        let text = render("all time", false, &l, &ReportBody::Data(aggregate(&rows)));
        assert!(text.contains("\"rate_pct\":null"));
    }

    #[test]
    fn concentration_null_below_minimum_sample() {
        let rows = vec![row("grep", 10, 10)];
        let l = load(1, 0, rows.clone());
        let text = render("all time", false, &l, &ReportBody::Data(aggregate(&rows)));
        assert!(text.contains("\"concentration\":null"));
    }

    #[test]
    fn program_names_are_escaped_via_shared_json_string() {
        let rows = vec![row("weird\"name", 10, 10)];
        let l = load(1, 0, rows.clone());
        let text = render("all time", false, &l, &ReportBody::Data(aggregate(&rows)));
        assert!(text.contains(r#"weird\"name"#));
    }
}
