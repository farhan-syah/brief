//! Pure aggregation over already-filtered `ReportRow`s: no file I/O, so
//! this is fully unit-testable without a filesystem.

use std::collections::HashMap;

use crate::fold::tokens::estimate_tokens_len;

use super::load::LoadResult;
use super::parse::ReportRow;

/// Row count and share of total raw bytes carried by one concentration
/// band (top 1% / 5% / 20% of calls by raw bytes).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ConcentrationBand {
    pub(crate) rows: usize,
    pub(crate) pct_of_bytes: f64,
}

/// How concentrated raw bytes are across calls, sorted by raw bytes
/// descending — the specific thing a flat savings percentage hides.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Concentration {
    pub(crate) top1: ConcentrationBand,
    pub(crate) top5: ConcentrationBand,
    pub(crate) top20: ConcentrationBand,
}

/// Re-read cost, per the double-counting correction: a count and a rate
/// against the number of folds, never a byte sum. See the comment at
/// `aggregate`'s computation site for why summing bytes here is wrong.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct RereadStats {
    pub(crate) reread_rows: usize,
    pub(crate) folded_rows: usize,
}

impl RereadStats {
    /// `None` when there were no folds at all — the rate has no
    /// denominator, and printing `0%` in that case would misreport
    /// "confirmed no cost" instead of "nothing to measure."
    pub(crate) fn rate(&self) -> Option<f64> {
        if self.folded_rows == 0 {
            None
        } else {
            Some(self.reread_rows as f64 / self.folded_rows as f64 * 100.0)
        }
    }
}

/// One program's row in the per-program table.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ProgramStats {
    pub(crate) program: String,
    pub(crate) calls: usize,
    pub(crate) raw_bytes: u64,
    pub(crate) kept_bytes: u64,
    pub(crate) folded_count: usize,
    pub(crate) reduction_pct: f64,
}

/// The numeric heart of the report: everything `render_text`/`render_json`
/// need, computed once from an already-filtered row set.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ReportSummary {
    pub(crate) row_count: usize,
    pub(crate) raw_bytes: u64,
    pub(crate) kept_bytes: u64,
    pub(crate) set_aside_pct: f64,
    pub(crate) raw_tokens: usize,
    pub(crate) kept_tokens: usize,
    /// `None` when there are fewer than 20 rows — too small a sample for
    /// a percentile breakdown to mean anything.
    pub(crate) concentration: Option<Concentration>,
    pub(crate) reread: RereadStats,
    /// Sorted by raw bytes descending, the same key as `concentration`
    /// uses, so the two views corroborate each other.
    pub(crate) programs: Vec<ProgramStats>,
    /// Context only, never folded into the savings math — a non-zero exit
    /// is routine for `grep`/`rg` (no match), not a failure.
    pub(crate) nonzero_exit_count: usize,
}

/// Minimum sample size below which a percentile breakdown is misleading
/// rather than informative.
const MIN_ROWS_FOR_CONCENTRATION: usize = 20;

/// Aggregate `rows` into a `ReportSummary`. Pure: takes no path, opens no
/// file, safe to call with zero rows (every field degrades to zero/`None`
/// rather than dividing by zero).
pub(crate) fn aggregate(rows: &[ReportRow]) -> ReportSummary {
    struct ProgramAcc {
        calls: usize,
        raw: u64,
        kept: u64,
        folded: usize,
    }

    let mut raw_bytes: u64 = 0;
    let mut kept_bytes: u64 = 0;
    let mut reread_rows = 0usize;
    let mut folded_rows = 0usize;
    let mut nonzero_exit_count = 0usize;
    let mut by_program: HashMap<&str, ProgramAcc> = HashMap::new();

    for row in rows {
        let row_raw = row.stdout_raw_bytes + row.stderr_raw_bytes;
        let row_kept = row.stdout_kept_bytes + row.stderr_kept_bytes;
        let row_folded = row.stdout_folded || row.stderr_folded;

        raw_bytes += row_raw;
        kept_bytes += row_kept;
        if row_folded {
            folded_rows += 1;
        }
        // Correction 1: never add re-read bytes into `kept`/`raw_bytes`.
        // A re-reading invocation is itself a fold-target row already
        // counted in the totals above (its own stdout_raw/kept_bytes), so
        // adding its bytes again here would double-count the exact bytes
        // the savings percentage is built from. The honest cost figure is
        // a rate against folds, computed below from `reread_rows` /
        // `folded_rows` — never a byte sum.
        if row.reads_fold {
            reread_rows += 1;
        }
        // Correction 2: a non-zero exit is not excluded from the totals
        // above — `grep`/`rg` exit 1 on "no match," a normal outcome, not
        // a failure. It is only ever counted here, for context.
        if row.exit_code != 0 {
            nonzero_exit_count += 1;
        }

        let acc = by_program
            .entry(row.program.as_str())
            .or_insert_with(|| ProgramAcc {
                calls: 0,
                raw: 0,
                kept: 0,
                folded: 0,
            });
        acc.calls += 1;
        acc.raw += row_raw;
        acc.kept += row_kept;
        if row_folded {
            acc.folded += 1;
        }
    }

    let mut programs: Vec<ProgramStats> = by_program
        .into_iter()
        .map(|(program, acc)| ProgramStats {
            program: program.to_string(),
            calls: acc.calls,
            raw_bytes: acc.raw,
            kept_bytes: acc.kept,
            folded_count: acc.folded,
            reduction_pct: reduction_pct(acc.raw, acc.kept),
        })
        .collect();
    // Same sort key as concentration (raw bytes descending) so the two
    // views corroborate each other; program name breaks ties so the order
    // is deterministic rather than HashMap-iteration-order flaky.
    programs.sort_by(|a, b| {
        b.raw_bytes
            .cmp(&a.raw_bytes)
            .then_with(|| a.program.cmp(&b.program))
    });

    let concentration = if rows.len() >= MIN_ROWS_FOR_CONCENTRATION {
        let mut per_row_raw: Vec<u64> = rows
            .iter()
            .map(|r| r.stdout_raw_bytes + r.stderr_raw_bytes)
            .collect();
        per_row_raw.sort_unstable_by(|a, b| b.cmp(a));
        Some(Concentration {
            top1: concentration_band(&per_row_raw, raw_bytes, 1.0),
            top5: concentration_band(&per_row_raw, raw_bytes, 5.0),
            top20: concentration_band(&per_row_raw, raw_bytes, 20.0),
        })
    } else {
        None
    };

    ReportSummary {
        row_count: rows.len(),
        raw_bytes,
        kept_bytes,
        set_aside_pct: reduction_pct(raw_bytes, kept_bytes),
        // Tokens applied once to the summed totals, never per row then
        // summed: `ceil` per row compounds into meaningful drift over
        // thousands of rows.
        raw_tokens: estimate_tokens_len(raw_bytes as usize),
        kept_tokens: estimate_tokens_len(kept_bytes as usize),
        concentration,
        reread: RereadStats {
            reread_rows,
            folded_rows,
        },
        programs,
        nonzero_exit_count,
    }
}

fn reduction_pct(raw: u64, kept: u64) -> f64 {
    if raw == 0 {
        0.0
    } else {
        (raw.saturating_sub(kept)) as f64 / raw as f64 * 100.0
    }
}

/// Bytes carried by the top `pct`% of `sorted_desc` (raw bytes per row,
/// sorted descending), as a share of `total_raw_bytes`. Always at least
/// one row, so "top 1%" of a 20-row sample means "the single largest
/// call," not zero.
fn concentration_band(sorted_desc: &[u64], total_raw_bytes: u64, pct: f64) -> ConcentrationBand {
    let n = sorted_desc.len();
    // `.min(n)` and `take` rather than a slice index: an empty sample must
    // yield an empty band, not a panic. Today the only caller guards on
    // n >= 20, but a private helper that panics on empty input is a
    // landmine for whoever calls it next.
    let k = ((n as f64 * pct / 100.0).ceil() as usize)
        .clamp(1, n.max(1))
        .min(n);
    let sum: u64 = sorted_desc.iter().take(k).sum();
    let pct_of_bytes = if total_raw_bytes == 0 {
        0.0
    } else {
        sum as f64 / total_raw_bytes as f64 * 100.0
    };
    ConcentrationBand {
        rows: k,
        pct_of_bytes,
    }
}

#[cfg(test)]
mod concentration_band_tests {
    use super::*;

    #[test]
    fn empty_sample_yields_an_empty_band_rather_than_panicking() {
        let band = concentration_band(&[], 0, 1.0);
        assert_eq!(band.rows, 0);
        assert_eq!(band.pct_of_bytes, 0.0);
    }

    #[test]
    fn top_band_always_covers_at_least_one_row() {
        let band = concentration_band(&[100, 1, 1], 102, 1.0);
        assert_eq!(band.rows, 1);
        assert!((band.pct_of_bytes - 100.0 * 100.0 / 102.0).abs() < 1e-9);
    }
}

/// What kind of report body to render, decided from `LoadResult` alone —
/// distinguishes "never recorded anything," "recorded but nothing
/// parsed," "recorded but nothing in this window," and "here's the data,"
/// so a reader never has to infer which one happened.
pub(crate) enum ReportBody {
    NoData,
    AllMalformed,
    EmptyAfterFilter,
    Data(ReportSummary),
}

/// Classify `load` and aggregate its rows if there are any to aggregate.
pub(crate) fn classify(load: &LoadResult) -> ReportBody {
    if load.total_lines == 0 {
        return ReportBody::NoData;
    }
    if load.parsed_total() == 0 {
        return ReportBody::AllMalformed;
    }
    if load.rows.is_empty() {
        return ReportBody::EmptyAfterFilter;
    }
    ReportBody::Data(aggregate(&load.rows))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(
        program: &str,
        raw: u64,
        kept: u64,
        folded: bool,
        reads_fold: bool,
        exit_code: i32,
    ) -> ReportRow {
        ReportRow {
            ts_ms: 0,
            program: program.to_string(),
            cwd: None,
            exit_code,
            stdout_raw_bytes: raw,
            stdout_kept_bytes: kept,
            stdout_folded: folded,
            stderr_raw_bytes: 0,
            stderr_kept_bytes: 0,
            stderr_folded: false,
            reads_fold,
        }
    }

    #[test]
    fn empty_rows_never_divides_by_zero() {
        let summary = aggregate(&[]);
        assert_eq!(summary.row_count, 0);
        assert_eq!(summary.set_aside_pct, 0.0);
        assert!(summary.concentration.is_none());
        assert_eq!(summary.reread.rate(), None);
        assert!(summary.programs.is_empty());
    }

    #[test]
    fn set_aside_pct_matches_raw_minus_kept_over_raw() {
        let rows = vec![row("grep", 1000, 200, true, false, 0)];
        let summary = aggregate(&rows);
        assert_eq!(summary.raw_bytes, 1000);
        assert_eq!(summary.kept_bytes, 200);
        assert!((summary.set_aside_pct - 80.0).abs() < 1e-9);
    }

    #[test]
    fn reread_rate_never_sums_bytes_only_counts_rows() {
        // Two folded rows, one of them also a re-read. Rate must be
        // 1/2 = 50%, entirely independent of how large either row's
        // bytes are — proving the rate is not a disguised byte ratio.
        let rows = vec![
            row("grep", 1_000_000, 1_000, true, true, 0),
            row("grep", 10, 10, true, false, 0),
        ];
        let summary = aggregate(&rows);
        assert_eq!(summary.reread.reread_rows, 1);
        assert_eq!(summary.reread.folded_rows, 2);
        assert_eq!(summary.reread.rate(), Some(50.0));
    }

    #[test]
    fn reread_rate_is_none_when_there_are_no_folds() {
        let rows = vec![row("grep", 10, 10, false, false, 0)];
        let summary = aggregate(&rows);
        assert_eq!(summary.reread.folded_rows, 0);
        assert_eq!(
            summary.reread.rate(),
            None,
            "no denominator must not print as 0%"
        );
    }

    #[test]
    fn nonzero_exit_is_counted_but_never_excluded_from_totals() {
        let rows = vec![
            row("grep", 100, 0, false, false, 1), // "no match" — routine
            row("grep", 100, 100, false, false, 0),
        ];
        let summary = aggregate(&rows);
        assert_eq!(summary.nonzero_exit_count, 1);
        assert_eq!(
            summary.raw_bytes, 200,
            "the exit-1 row's bytes must still count"
        );
    }

    #[test]
    fn concentration_is_none_below_the_minimum_sample_size() {
        let rows: Vec<ReportRow> = (0..19)
            .map(|i| row("grep", i, i, false, false, 0))
            .collect();
        let summary = aggregate(&rows);
        assert!(summary.concentration.is_none());
    }

    #[test]
    fn concentration_present_at_the_minimum_sample_size() {
        let mut rows: Vec<ReportRow> = (0..19)
            .map(|_| row("grep", 1, 1, false, false, 0))
            .collect();
        rows.push(row("grep", 1000, 1000, false, false, 0)); // 20th, dominates
        let summary = aggregate(&rows);
        let conc = summary
            .concentration
            .expect("20 rows must produce a concentration breakdown");
        assert_eq!(conc.top1.rows, 1);
        assert!(
            conc.top1.pct_of_bytes > 90.0,
            "one dominant row must carry nearly all the bytes"
        );
    }

    #[test]
    fn programs_sorted_by_raw_bytes_descending() {
        let rows = vec![
            row("cat", 10, 10, false, false, 0),
            row("grep", 1000, 1000, false, false, 0),
            row("find", 100, 100, false, false, 0),
        ];
        let summary = aggregate(&rows);
        let names: Vec<&str> = summary
            .programs
            .iter()
            .map(|p| p.program.as_str())
            .collect();
        assert_eq!(names, vec!["grep", "find", "cat"]);
    }

    #[test]
    fn per_program_folded_count_and_reduction_pct() {
        let rows = vec![
            row("grep", 1000, 200, true, false, 0),
            row("grep", 1000, 1000, false, false, 0),
        ];
        let summary = aggregate(&rows);
        let grep = &summary.programs[0];
        assert_eq!(grep.calls, 2);
        assert_eq!(grep.folded_count, 1);
        assert_eq!(grep.raw_bytes, 2000);
        assert_eq!(grep.kept_bytes, 1200);
        assert!((grep.reduction_pct - 40.0).abs() < 1e-9);
    }
}
