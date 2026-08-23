//! The report's two standing caveats, in both of their renderings.
//!
//! Text and JSON word these differently on purpose. The terminal form is
//! attached inline to the number it qualifies, so it must stay short enough
//! not to bury the number. The JSON form is read by a consumer that may
//! show it on its own, with no adjacent number to lend it context, so it
//! states the whole fact.
//!
//! They live in one file because they are two renderings of the SAME fact.
//! Split across the renderers, an edit to the meaning changes one and
//! leaves the other quietly stating something no longer true — and a
//! caveat that has drifted from what it qualifies is worse than none.

/// Inline form: what the totals actually cover.
pub(crate) const SCOPE_SHORT: &str =
    "grep/cat/find/rg only — output brief handled, not your total usage";

/// Standalone form of the same fact.
pub(crate) const SCOPE_FULL: &str = "Scope: only grep, cat, find, and rg calls are tracked, so every \
     number is output brief handled, never total output or your token usage.";

/// Inline form: why the re-read count is a floor, not a measurement.
pub(crate) const LOWER_BOUND_SHORT: &str =
    "lower bound: only catches a re-read that goes back through brief's own argv";

/// Standalone form of the same fact.
pub(crate) const LOWER_BOUND_FULL: &str = "Lower bound: the re-read count only catches a re-read that \
     goes back through brief's own argv — including the `brief tail ...` the fold hint prescribes. \
     A re-read that bypasses brief entirely (a plain shell tail, an editor, a pager) is invisible to it.";

#[cfg(test)]
mod tests {
    use super::*;

    /// The short and full forms must keep naming the same things. If an
    /// edit drops a target from one list, this fails rather than leaving
    /// the two forms describing different scopes.
    #[test]
    fn both_scope_forms_name_every_tracked_program() {
        for target in ["grep", "cat", "find", "rg"] {
            assert!(SCOPE_SHORT.contains(target), "short form lost {target}");
            assert!(SCOPE_FULL.contains(target), "full form lost {target}");
        }
    }

    #[test]
    fn both_lower_bound_forms_name_the_signal_they_are_limited_to() {
        assert!(LOWER_BOUND_SHORT.contains("argv"));
        assert!(LOWER_BOUND_FULL.contains("argv"));
    }

    #[test]
    fn short_forms_stay_short_enough_to_sit_inline() {
        // They are appended to a line that already carries a number; past
        // roughly this width the caveat buries what it qualifies.
        assert!(SCOPE_SHORT.len() <= 80, "scope note too long to inline");
        assert!(
            LOWER_BOUND_SHORT.len() <= 80,
            "lower-bound note too long to inline"
        );
    }
}
