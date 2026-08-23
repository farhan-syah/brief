//! `brief init`: install or uninstall brief's PreToolUse hook in the
//! user's Claude Code settings.json, or generate/remove PATH shims
//! (`--shims <dir>`) that work with any harness. Wiring only — see `cli`
//! for argv parsing, `fs_ops` for home-directory resolution, backup,
//! atomic write, and `--dry-run` gating (hook install/uninstall),
//! `settings_edit` for the pure text transform against a known JSON
//! shape, `shims` for the pure shim-script template and marker, and
//! `shim_fs` for writing/removing shim files on disk.
//!
//! This edits a user's live config or PATH-visible files, so it is held
//! to a higher bar than the rest of the crate: never a hardcoded home
//! directory (tests inject it), never a partial write (backup first,
//! write atomically), never a guess at an unrecognized settings.json
//! shape (refuse and print the block for the user to paste by hand — see
//! `settings_edit`'s module doc), and shim uninstall only ever removes a
//! file carrying brief's own marker (see `shims`).

mod cli;
mod fs_ops;
mod settings_edit;
mod shim_fs;
mod shims;

pub(crate) use cli::run;
