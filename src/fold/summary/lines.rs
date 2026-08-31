//! Line selection and counting for fold summaries.

/// Total line count of `bytes`, defined as the newline count plus a final
/// unterminated line if there is one. Identical to `str::lines().count()`
/// for valid UTF-8, but computable from a running counter, which is what
/// the streaming runner needs — it counts `\n` per chunk and never
/// materializes a line.
pub(crate) fn count_lines(bytes: &[u8]) -> usize {
    total_lines_from(
        bytes.iter().filter(|b| **b == b'\n').count(),
        bytes.last().copied(),
    )
}

/// `count_lines` in incremental form: newlines seen so far plus the last
/// byte seen. The streaming runner keeps exactly these two values.
pub(crate) fn total_lines_from(newlines: usize, last_byte: Option<u8>) -> usize {
    match last_byte {
        Some(b'\n') | None => newlines,
        // Trailing bytes after the last newline are one more (unterminated) line.
        Some(_) => newlines + 1,
    }
}

/// First `n` lines of `bytes`, without the newline that terminates line `n`.
/// Returns everything when `bytes` holds fewer than `n` newlines — including
/// a trailing partial line, which is what a byte-bounded head window has.
pub(super) fn first_lines(bytes: &[u8], n: usize) -> &[u8] {
    if n == 0 {
        return &[];
    }
    let mut seen = 0;
    for (i, b) in bytes.iter().enumerate() {
        if *b == b'\n' {
            seen += 1;
            if seen == n {
                return &bytes[..i];
            }
        }
    }
    bytes
}

/// Last `n` lines of `bytes`, without a trailing newline. One trailing
/// newline is dropped first so `"a\nb\n"` with `n == 1` yields `"b"`, not an
/// empty final line.
pub(super) fn last_lines(bytes: &[u8], n: usize) -> &[u8] {
    if n == 0 {
        return &[];
    }
    let end = match bytes.last() {
        Some(b'\n') => bytes.len() - 1,
        _ => bytes.len(),
    };
    let body = &bytes[..end];
    let mut seen = 0;
    for i in (0..body.len()).rev() {
        if body[i] == b'\n' {
            seen += 1;
            if seen == n {
                return &body[i + 1..];
            }
        }
    }
    body
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_lines_matches_str_lines_count() {
        for raw in ["", "a", "a\n", "a\nb", "a\nb\n", "a\n\n", "\n", "\n\n"] {
            assert_eq!(
                count_lines(raw.as_bytes()),
                raw.lines().count(),
                "count_lines disagrees with str::lines on {raw:?}"
            );
        }
    }

    #[test]
    fn first_and_last_lines_select_without_terminators() {
        let raw = b"a\nb\nc\n";
        assert_eq!(first_lines(raw, 0), b"");
        assert_eq!(first_lines(raw, 2), b"a\nb");
        assert_eq!(
            first_lines(raw, 9),
            raw.as_slice(),
            "fewer lines than asked: take all"
        );
        assert_eq!(last_lines(raw, 0), b"");
        assert_eq!(last_lines(raw, 1), b"c");
        assert_eq!(last_lines(raw, 2), b"b\nc");
        assert_eq!(last_lines(b"a\nb", 1), b"b");
    }
}
