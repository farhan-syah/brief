//! `brief init`: install or uninstall brief's PreToolUse hook in the
//! user's Claude Code settings.json. Wiring only — see `cli` for argv
//! parsing, `fs_ops` for home-directory resolution, backup, atomic write,
//! and `--dry-run` gating, and `settings_edit` for the pure text
//! transform against a known JSON shape.
//!
//! This edits a user's live config, so it is held to a higher bar than
//! the rest of the crate: never a hardcoded home directory (tests inject
//! it), never a partial write (backup first, write atomically), and never
//! a guess at an unrecognized shape (refuse and print the block for the
//! user to paste by hand — see `settings_edit`'s module doc).

mod cli;
mod fs_ops;
mod settings_edit;

pub(crate) use cli::run;
