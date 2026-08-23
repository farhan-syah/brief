//! Read the tracking file into `ReportRow`s, streaming line by line rather
//! than loading the whole file into memory.
//!
//! The tracking file is read as raw bytes and split on `\n`, never decoded
//! as UTF-8 up front — mirroring `track::retention::compact`'s reasoning:
//! a single corrupt byte anywhere in the file must make that one line
//! malformed, not fail the whole report.

use std::fs;
use std::io::{self, BufRead};
use std::path::Path;

use super::parse::{ReportRow, parse_line};

/// Outcome of reading and filtering the tracking file. `rows` holds only
/// the rows that both parsed AND fell inside the `--since`/`--project`
/// filters — a row can be excluded from `rows` for either reason, and
/// `malformed` + `rows.len()` do not have to sum to `total_lines`: the gap
/// is rows that parsed fine but were filtered out.
pub(crate) struct LoadResult {
    pub(crate) rows: Vec<ReportRow>,
    pub(crate) malformed: usize,
    /// Every non-blank line seen in the file, whether it parsed or not.
    pub(crate) total_lines: usize,
}

impl LoadResult {
    /// Lines that parsed successfully, whether or not they survived the
    /// filters — i.e. `total_lines - malformed`.
    pub(crate) fn parsed_total(&self) -> usize {
        self.total_lines - self.malformed
    }
}

/// Read `path`, keeping rows with `ts_ms >= since_ms` (when given) and,
/// when `project_cwd` is given, `cwd == Some(project_cwd)`. `since_ms` of
/// `None` means "all time": no time filter at all.
pub(crate) fn load_rows(
    path: &Path,
    since_ms: Option<u128>,
    project_cwd: Option<&str>,
) -> io::Result<LoadResult> {
    let file = fs::File::open(path)?;
    let mut reader = io::BufReader::new(file);
    let mut rows = Vec::new();
    let mut malformed = 0usize;
    let mut total_lines = 0usize;
    let mut buf: Vec<u8> = Vec::new();

    loop {
        buf.clear();
        let n = reader.read_until(b'\n', &mut buf)?;
        if n == 0 {
            break;
        }
        while matches!(buf.last(), Some(b'\n') | Some(b'\r')) {
            buf.pop();
        }
        if buf.is_empty() {
            continue; // a blank line is not a row, not counted either way
        }
        total_lines += 1;

        let Ok(line) = std::str::from_utf8(&buf) else {
            malformed += 1;
            continue;
        };
        let Some(row) = parse_line(line) else {
            malformed += 1;
            continue;
        };

        if let Some(cutoff) = since_ms
            && row.ts_ms < cutoff
        {
            continue; // filtered: outside the window, not malformed
        }
        if let Some(want_cwd) = project_cwd
            && row.cwd.as_deref() != Some(want_cwd)
        {
            continue; // filtered: not this project directory
        }

        rows.push(row);
    }

    Ok(LoadResult {
        rows,
        malformed,
        total_lines,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::track::InvocationRecord;
    use std::io::Write as _;

    fn record(ts_ms: u128, cwd: Option<&str>) -> InvocationRecord {
        InvocationRecord {
            ts_ms,
            program: "grep".to_string(),
            args: "-r foo .".to_string(),
            cwd: cwd.map(str::to_string),
            exit_code: 0,
            exec_time_ms: 1,
            stdout_raw_bytes: Some(100),
            stdout_kept_bytes: Some(100),
            stdout_folded: false,
            stdout_path: None,
            stderr_raw_bytes: Some(0),
            stderr_kept_bytes: Some(0),
            stderr_folded: false,
            stderr_path: None,
            reads_fold: false,
            captured: true,
        }
    }

    fn write_lines(lines: &[String]) -> tempfile::TempPath {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        for line in lines {
            f.write_all(line.as_bytes()).unwrap();
        }
        f.flush().unwrap();
        f.into_temp_path()
    }

    #[test]
    fn empty_file_yields_zero_everything() {
        let path = write_lines(&[]);
        let result = load_rows(&path, None, None).unwrap();
        assert_eq!(result.total_lines, 0);
        assert_eq!(result.malformed, 0);
        assert!(result.rows.is_empty());
    }

    #[test]
    fn malformed_lines_are_counted_not_dropped_silently() {
        let path = write_lines(&[
            record(1000, None).to_line(),
            "not json\n".to_string(),
            record(2000, None).to_line(),
        ]);
        let result = load_rows(&path, None, None).unwrap();
        assert_eq!(result.total_lines, 3);
        assert_eq!(result.malformed, 1);
        assert_eq!(result.rows.len(), 2);
        assert_eq!(result.parsed_total(), 2);
    }

    #[test]
    fn invalid_utf8_line_is_malformed_not_fatal() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(record(1000, None).to_line().as_bytes())
            .unwrap();
        f.write_all(&[0xff, 0xfe, 0x80]).unwrap();
        f.write_all(b"\n").unwrap();
        f.write_all(record(2000, None).to_line().as_bytes())
            .unwrap();
        f.flush().unwrap();
        let path = f.into_temp_path();

        let result = load_rows(&path, None, None).unwrap();
        assert_eq!(result.total_lines, 3);
        assert_eq!(result.malformed, 1);
        assert_eq!(result.rows.len(), 2);
    }

    #[test]
    fn since_filters_out_older_rows_without_counting_them_malformed() {
        let path = write_lines(&[record(1000, None).to_line(), record(5000, None).to_line()]);
        let result = load_rows(&path, Some(3000), None).unwrap();
        assert_eq!(result.total_lines, 2);
        assert_eq!(result.malformed, 0, "filtered rows are not malformed");
        assert_eq!(result.rows.len(), 1);
        assert_eq!(result.rows[0].ts_ms, 5000);
    }

    #[test]
    fn project_filters_to_matching_cwd_only() {
        let path = write_lines(&[
            record(1000, Some("/a")).to_line(),
            record(2000, Some("/b")).to_line(),
            record(3000, None).to_line(),
        ]);
        let result = load_rows(&path, None, Some("/a")).unwrap();
        assert_eq!(result.rows.len(), 1);
        assert_eq!(result.rows[0].cwd.as_deref(), Some("/a"));
    }

    #[test]
    fn none_since_means_all_time() {
        let path = write_lines(&[record(1, None).to_line()]);
        let result = load_rows(&path, None, None).unwrap();
        assert_eq!(result.rows.len(), 1);
    }
}
