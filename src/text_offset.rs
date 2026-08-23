//! One small pointer-arithmetic utility, promoted here once a second
//! quote-aware JSON scanner (`init::settings_edit`, alongside
//! `hook::protocol`) needed it — see `private_fs` for the same pattern
//! applied to filesystem helpers.

/// The byte offset of subslice `sub` within `parent`, or `None` if `sub`
/// falls outside `parent`'s address range. Compares addresses without
/// dereferencing either pointer, so this is safe code.
///
/// PRECONDITION: `sub` must be a slice derived from `parent`. The range
/// check is a guard against a caller mixing up two buffers, not proof of
/// provenance — an unrelated allocation that happens to sit inside
/// `parent`'s range would be reported as an offset. Every caller here
/// slices the same string it passes as `parent`, which is what makes the
/// result meaningful.
pub(crate) fn offset_in(parent: &str, sub: &str) -> Option<usize> {
    let p = parent.as_ptr() as usize;
    let s = sub.as_ptr() as usize;
    if s < p || s > p + parent.len() {
        return None;
    }
    Some(s - p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_offset_of_a_real_subslice() {
        let parent = "hello world";
        let sub = &parent[6..11];
        assert_eq!(offset_in(parent, sub), Some(6));
    }

    #[test]
    fn empty_subslice_at_the_end_is_a_valid_offset() {
        let parent = "hello";
        let sub = &parent[5..5];
        assert_eq!(offset_in(parent, sub), Some(5));
    }

    #[test]
    fn a_buffer_outside_the_parent_range_is_rejected() {
        // Guards the caller-mixed-up-two-buffers case. It cannot reject an
        // unrelated allocation that happens to land inside `parent`'s
        // range — string literals in particular are laid out adjacently by
        // the compiler — which is why the precondition on `offset_in` is
        // that the caller derived `sub` from `parent`.
        let parent = String::from("hello world");
        let other = String::from("a totally separate heap allocation");
        assert_eq!(offset_in(&parent, &other), None);
    }
}
