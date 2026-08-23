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

/// Inline form: what the totals actually cover. Names the programs rather
/// than saying "tracked programs", because the whole job of this caveat is
/// letting a reader tell whether their command is in the number — a
/// generic phrase fits the width cap by dropping the one fact it exists to
/// carry. The tail is terse to buy room for the names.
pub(crate) fn scope_short() -> String {
    format!(
        "{} only — not your total usage",
        crate::targets::slash_list()
    )
}

/// Standalone form of the same fact, naming every tracked program from
/// the single shared `crate::targets::TARGETS` list.
pub(crate) fn scope_full() -> String {
    format!(
        "Scope: only {} calls are tracked, so every number is output brief \
         handled, never total output or your token usage.",
        crate::targets::oxford_list()
    )
}

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

    /// `scope_full` must keep naming every tracked program. If an edit
    /// drops a target from `crate::targets::TARGETS`, this fails rather
    /// than leaving the full form describing a scope narrower than
    /// what's actually tracked.
    #[test]
    fn scope_full_names_every_tracked_program() {
        let full = scope_full();
        for target in crate::targets::TARGETS {
            assert!(full.contains(target), "full form lost {target}");
        }
    }

    /// The inline form must name every tracked program: a reader has to be
    /// able to tell whether their own command is inside the number it is
    /// attached to.
    #[test]
    fn scope_short_names_every_tracked_program() {
        let short = scope_short();
        for t in crate::targets::TARGETS {
            assert!(short.contains(t), "inline scope note lost {t}");
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
        assert!(
            scope_short().len() <= 80,
            "scope note too long to sit inline: {}",
            scope_short()
        );
        assert!(
            LOWER_BOUND_SHORT.len() <= 80,
            "lower-bound note too long to inline"
        );
    }
}
