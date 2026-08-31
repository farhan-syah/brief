//! Ported verbatim from rtk's `core::tracking::estimate_tokens` — kept
//! byte-identical so ogt's fold-trigger numbers stay comparable to
//! rtk's historical token-savings database.
//! Source: reference/rtk/src/core/tracking.rs

/// Estimate token count from a byte count: ~4 chars/token, rounded up so a
/// borderline output is never under-counted into passthrough.
///
/// Takes a length rather than text so the streaming runner can gate on a
/// running byte counter — it sees arbitrary bytes, never a `str`, and must
/// decide before it has (or ever will have) a UTF-8 view of the output.
pub(crate) fn estimate_tokens_len(bytes: usize) -> usize {
    (bytes as f64 / 4.0).ceil() as usize
}

/// Estimate token count of `text` from its byte length.
pub(crate) fn estimate_tokens(text: &str) -> usize {
    estimate_tokens_len(text.len())
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

    #[test]
    fn len_wrapper_agrees_with_text_form() {
        for n in [0usize, 1, 4, 5, 99_993, 100_000] {
            assert_eq!(estimate_tokens_len(n), estimate_tokens(&"x".repeat(n)));
        }
    }
}
