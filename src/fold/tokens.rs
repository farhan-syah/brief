//! Ported verbatim from rtk's `core::tracking::estimate_tokens` — kept
//! byte-identical so sigfold's fold-trigger numbers stay comparable to
//! rtk's historical token-savings database.
//! Source: reference/rtk/src/core/tracking.rs

/// Estimate token count from byte length: ~4 chars/token, rounded up so a
/// borderline output is never under-counted into passthrough.
pub(crate) fn estimate_tokens(text: &str) -> usize {
    (text.len() as f64 / 4.0).ceil() as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_is_zero() {
        assert_eq!(estimate_tokens(""), 0);
    }

    #[test]
    fn rounds_up_to_next_token() {
        assert_eq!(estimate_tokens("a"), 1);
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("abcde"), 2);
    }

    #[test]
    fn matches_rtk_reference_values() {
        // ceil(len / 4), same formula as rtk core::tracking::estimate_tokens.
        assert_eq!(estimate_tokens(&"x".repeat(100_000)), 25_000);
        assert_eq!(estimate_tokens(&"x".repeat(99_999)), 25_000);
        assert_eq!(estimate_tokens(&"x".repeat(99_996)), 24_999);
        assert_eq!(estimate_tokens(&"x".repeat(99_993)), 24_999);
        assert_eq!(estimate_tokens(&"x".repeat(99_992)), 24_998);
    }
}
