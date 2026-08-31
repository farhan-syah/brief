//! Pure text/shape logic for PATH shims: the shim script template, and the
//! marker ogt uses to recognize its own generated files. No filesystem
//! I/O here — see `init::shim_fs` for that, matching `settings_edit`'s
//! split from `fs_ops`.
//!
//! Every shim references ogt by the absolute path of the currently
//! running executable, so it keeps working regardless of `PATH` order —
//! never a bare `ogt`, which the shim's own directory could shadow. Each
//! shim also exports `OGT_SHIM_DIR` to its own directory before invoking
//! ogt; see `crate::cli::path_shim` for why that is required to avoid
//! infinite recursion.

/// Fixed marker every ogt-generated shim carries, as its own comment
/// line — so uninstall can tell ogt's own files apart from anything
/// else a user put in the shim directory. Checked as an exact-line match,
/// never a substring guess.
const MARKER: &str = "ogt shim v1";

/// Render one shim script for `program`, invoking `ogt_exe` (the
/// absolute path of the running ogt binary, from
/// `std::env::current_exe()`).
pub(crate) fn render_shim(ogt_exe: &str, program: &str) -> String {
    use crate::cli::OGT_SHIM_DIR as VAR;
    format!(
        "#!/bin/sh\n\
         # {MARKER}\n\
         {VAR}=\"$(cd \"$(dirname \"$0\")\" && pwd)\"\n\
         export {VAR}\n\
         exec \"{ogt_exe}\" {program} \"$@\"\n"
    )
}

/// `true` if `contents` is a shim ogt generated — an exact-line match on
/// the marker comment, never a loose substring check that could misfire
/// on a user's own comment.
pub(crate) fn is_ogt_shim(contents: &str) -> bool {
    let marker_line = format!("# {MARKER}");
    contents.lines().any(|line| line.trim() == marker_line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_shim_carries_marker_and_absolute_ogt_path() {
        let script = render_shim("/abs/path/to/ogt", "grep");
        assert!(script.starts_with("#!/bin/sh\n"));
        assert!(is_ogt_shim(&script));
        assert!(script.contains("exec \"/abs/path/to/ogt\" grep \"$@\""));
        assert!(script.contains("OGT_SHIM_DIR"));
    }

    #[test]
    fn render_shim_is_specific_to_its_program() {
        let grep = render_shim("/abs/ogt", "grep");
        let cat = render_shim("/abs/ogt", "cat");
        assert_ne!(grep, cat);
        assert!(grep.contains(" grep \"$@\""));
        assert!(cat.contains(" cat \"$@\""));
    }

    #[test]
    fn is_ogt_shim_rejects_unmarked_content() {
        assert!(!is_ogt_shim("#!/bin/sh\necho hi\n"));
    }

    #[test]
    fn is_ogt_shim_rejects_a_loose_substring_match() {
        assert!(!is_ogt_shim(
            "#!/bin/sh\necho 'ogt shim v1 mentioned here'\n"
        ));
    }
}
