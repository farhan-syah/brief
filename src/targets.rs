//! The one list of programs ogt folds — matched on the basename of
//! `argv[1]` by `cli::dispatch::is_fold_target` and on the first
//! remaining word's basename by `hook::decide::rewrite`. Both modules
//! import this constant instead of keeping their own copy: two
//! independent lists that happen to agree today is exactly the shape of
//! bug that goes silent — a name present in one but not the other either
//! skips the gate entirely or gets rewritten into a dead no-op path. See
//! `hook::decide`'s tests for the regression guard that checks the two
//! real decision functions agree on every name here.

/// Programs ogt folds. Everything else takes the passthrough path
/// (`cli::dispatch`) or is left alone untouched (`hook::decide`).
pub(crate) const TARGETS: [&str; 8] = ["grep", "cat", "find", "rg", "cargo", "git", "ls", "diff"];

/// Oxford-comma sentence of `TARGETS`, e.g.
/// "grep, cat, find, rg, cargo, git, ls, and diff".
pub(crate) fn oxford_list() -> String {
    match TARGETS.split_last() {
        None => String::new(),
        Some((last, [])) => (*last).to_string(),
        Some((last, rest)) => format!("{}, and {last}", rest.join(", ")),
    }
}

/// Slash-joined list of `TARGETS`, e.g. "grep/cat/find/rg/cargo/git/ls/diff"
/// — used in running prose where an Oxford-comma sentence would read as a
/// list rather than a compact scope tag.
pub(crate) fn slash_list() -> String {
    TARGETS.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oxford_list_names_every_target_and_ends_with_a_final_and() {
        let list = oxford_list();
        for t in TARGETS {
            assert!(list.contains(t), "oxford list lost {t}");
        }
        assert!(list.ends_with("and diff"), "got: {list}");
    }

    #[test]
    fn slash_list_joins_every_target_with_slashes() {
        assert_eq!(slash_list(), "grep/cat/find/rg/cargo/git/ls/diff");
    }
}
