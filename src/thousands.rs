//! Digit grouping, shared by the fold summary and the report renderer.

/// Render `n` with `,` every three digits (`4131` -> `4,131`). Both the
/// fold summary's omitted-line count and the report's byte/token totals
/// are numbers a reader scans rather than computes with, and an unbroken
/// run of digits is where a misread order of magnitude comes from.
pub(crate) fn with_thousands_separator(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_every_three_digits_from_the_right() {
        assert_eq!(with_thousands_separator(0), "0");
        assert_eq!(with_thousands_separator(51), "51");
        assert_eq!(with_thousands_separator(999), "999");
        assert_eq!(with_thousands_separator(1000), "1,000");
        assert_eq!(with_thousands_separator(4131), "4,131");
        assert_eq!(with_thousands_separator(485397), "485,397");
        assert_eq!(with_thousands_separator(1234567), "1,234,567");
    }
}
