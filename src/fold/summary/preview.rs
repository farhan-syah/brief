//! UTF-8-safe preview slicing for fold summaries.

/// Byte cap per kept slice (head/tail), guarding against a single huge
/// line (e.g. minified JSON) defeating the line-based split and making
/// the "compact" fold as large as the raw output.
pub(crate) const SLICE_MAX_BYTES: usize = 8_000;

/// Decode at most `max_bytes` from the start of `bytes` for display.
///
/// A multi-byte char straddling the cap is dropped rather than turned into
/// U+FFFD, so a clean cut of valid UTF-8 stays clean. Genuinely invalid
/// bytes still decode lossily — the preview must never fail on binary
/// output — and the result is re-capped afterwards because each U+FFFD is
/// wider than the byte it replaces.
pub(super) fn lossy_prefix(bytes: &[u8], max_bytes: usize) -> String {
    let slice = &bytes[..max_bytes.min(bytes.len())];
    let end = match std::str::from_utf8(slice) {
        Ok(_) => slice.len(),
        // error_len None means "valid so far, sequence cut off at the end".
        Err(e) if e.error_len().is_none() => e.valid_up_to(),
        Err(_) => slice.len(),
    };
    let text = String::from_utf8_lossy(&slice[..end]).into_owned();
    if text.len() <= max_bytes {
        return text;
    }
    take_prefix_bytes(&text, max_bytes)
}

/// Decode at most `max_bytes` from the end of `bytes` for display. Mirror of
/// `lossy_prefix`: leading continuation bytes left by the cut are dropped.
pub(super) fn lossy_suffix(bytes: &[u8], max_bytes: usize) -> String {
    let start = bytes.len().saturating_sub(max_bytes);
    let mut slice = &bytes[start..];
    // A UTF-8 continuation byte is 0b10xxxxxx and can never start a char; at
    // most three of them precede the first real boundary. Applied even at
    // start == 0, because a byte-bounded tail window can itself begin
    // mid-char. Well-formed text is unaffected.
    let skip = slice
        .iter()
        .take(3)
        .take_while(|b| (**b & 0b1100_0000) == 0b1000_0000)
        .count();
    slice = &slice[skip..];
    let text = String::from_utf8_lossy(slice).into_owned();
    if text.len() <= max_bytes {
        return text;
    }
    take_suffix_bytes(&text, max_bytes)
}

/// Keep at most `max_bytes` from the start of `s`, cut on a UTF-8 char
/// boundary. Adapted from rtk's truncation-boundary logic in
/// `write_tee_file` (reference/rtk/src/core/tee.rs) — reused here only
/// for the in-memory summary, never for the persisted file.
fn take_prefix_bytes(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    // Walk DOWN to the nearest boundary at or below the cap. Taking every char
    // that starts below the cap and then adding its full width overshoots by up
    // to three bytes whenever a multi-byte char straddles the limit.
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

/// Keep at most `max_bytes` from the end of `s`, cut on a UTF-8 char
/// boundary.
fn take_suffix_bytes(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let min_start = s.len() - max_bytes;
    let boundary = s
        .char_indices()
        .find(|(i, _)| *i >= min_start)
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    s[boundary..].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn take_prefix_and_suffix_bytes_respect_utf8_boundaries() {
        let japanese = "\u{6F22}".repeat(333); // 999 bytes, 3-byte chars
        let prefix = take_prefix_bytes(&japanese, 998);
        assert!(prefix.len() <= 998);
        assert!(japanese.starts_with(&prefix));

        let suffix = take_suffix_bytes(&japanese, 998);
        assert!(suffix.len() <= 998);
        assert!(japanese.ends_with(&suffix));
    }

    #[test]
    fn lossy_slices_never_exceed_the_cap_on_invalid_utf8() {
        // Every byte is invalid UTF-8 and expands to a 3-byte U+FFFD.
        let junk = vec![0xffu8; 100];
        assert!(lossy_prefix(&junk, 12).len() <= 12);
        assert!(lossy_suffix(&junk, 12).len() <= 12);
    }

    #[test]
    fn lossy_prefix_drops_a_char_cut_by_the_cap() {
        let text = "ab\u{6F22}cd"; // 'a','b', 3-byte char, 'c','d'
        // Cap 3 lands inside the multi-byte char: drop it, do not pad U+FFFD.
        assert_eq!(lossy_prefix(text.as_bytes(), 3), "ab");
    }
}
