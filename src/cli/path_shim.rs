//! PATH-shim spawn resolution: when `BRIEF_SHIM_DIR` names a directory of
//! brief's own wrapper scripts (`brief init --shims <dir>`), a plain
//! `Command::new(program)` would resolve `program` back through that same
//! shim directory and loop forever — the shim invokes `brief <program>`,
//! `brief` looks up `<program>` on `PATH`, finds the shim again, and hangs
//! with zero output and no tracking row. The fix: before spawning, brief
//! computes an effective `PATH` with every occurrence of `BRIEF_SHIM_DIR`
//! removed, resolves `program` against that reduced `PATH`, and spawns the
//! resolved absolute path — and passes the same reduced `PATH` to the
//! child, so a wrapped `cargo` that itself spawns `git` gets the real
//! `git`, never another shim.
//!
//! Every function here is pure — `PATH`/shim-dir/program are plain values,
//! never read from the environment — so tests never touch the real
//! process environment, matching `fold::config` and `track::paths`.
//! `cli::dispatch::main_with` is the one caller that reads the real
//! `BRIEF_SHIM_DIR`/`PATH` env vars and feeds them in.

use std::env;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

/// Name of the env var a shim exports to its own directory, and the one
/// `resolve_spawn`'s caller reads. Shared as a constant — not duplicated as
/// a literal in both the shim script template (`init::shims::render_shim`)
/// and here — so the two sides can never drift out of agreement, which
/// would silently defeat the recursion guard this module exists for.
pub(crate) const BRIEF_SHIM_DIR: &str = "BRIEF_SHIM_DIR";

/// Remove every occurrence of `shim_dir` from `path`, preserving order and
/// every other entry untouched. A no-op — `path` returned unchanged — when
/// `shim_dir` is `None`. `PATH` entries are OS strings and may be
/// non-UTF-8; an empty entry (meaning "current directory") is preserved
/// unless it is itself the shim dir being removed.
pub(crate) fn reduce_path(path: &OsStr, shim_dir: Option<&OsStr>) -> OsString {
    let Some(shim_dir) = shim_dir else {
        return path.to_os_string();
    };
    let kept = env::split_paths(path).filter(|entry| entry.as_os_str() != shim_dir);
    env::join_paths(kept).unwrap_or_else(|_| path.to_os_string())
}

/// Resolve `program` to an absolute path by searching `path` the way a
/// shell would. Returns `None` — meaning "spawn as given, no resolution
/// needed or possible" — in two cases: `program` already names a path
/// (absolute like `/usr/bin/grep`, or relative like `./x` — either way it
/// has a directory component), or no directory in `path` holds an
/// executable file by that name.
pub(crate) fn resolve_program(program: &OsStr, path: &OsStr) -> Option<PathBuf> {
    let as_path = Path::new(program);
    let has_dir_component = as_path
        .parent()
        .is_some_and(|parent| !parent.as_os_str().is_empty());
    if has_dir_component {
        return None;
    }
    env::split_paths(path)
        .map(|dir| dir.join(as_path))
        .find(|candidate| is_executable_file(candidate))
}

#[cfg(unix)]
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable_file(path: &Path) -> bool {
    path.is_file()
}

/// Compute what to spawn and what `PATH` to give the child, given the raw
/// `BRIEF_SHIM_DIR` and `PATH` values (or their absence). When
/// `shim_dir`/`path` is `None` — no shims installed, or no `PATH` at all —
/// returns `(program, None)` unchanged: the caller must then leave the
/// child's environment untouched, exactly as before this feature existed.
/// When resolution fails (the program truly isn't found anywhere in the
/// reduced `PATH`), the original `program` is still returned unresolved so
/// the existing spawn-failure path (127/126) is reached exactly as today —
/// but the reduced `PATH` is still handed to the child, so its own lookup
/// agrees with the one just done here.
pub(crate) fn resolve_spawn(
    program: &OsStr,
    shim_dir: Option<&OsStr>,
    path: Option<&OsStr>,
) -> (OsString, Option<OsString>) {
    let (Some(shim_dir), Some(path)) = (shim_dir, path) else {
        return (program.to_os_string(), None);
    };
    let reduced = reduce_path(path, Some(shim_dir));
    let resolved = resolve_program(program, &reduced)
        .map(PathBuf::into_os_string)
        .unwrap_or_else(|| program.to_os_string());
    (resolved, Some(reduced))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn make_exec(dir: &Path, name: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        std::fs::write(&path, "#!/bin/sh\necho hi\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[test]
    fn reduce_path_is_a_no_op_when_shim_dir_unset() {
        let path = OsStr::new("/a:/b:/c");
        assert_eq!(reduce_path(path, None), OsString::from("/a:/b:/c"));
    }

    #[test]
    fn reduce_path_removes_every_occurrence_and_preserves_order() {
        let path = OsStr::new("/shim:/a:/shim:/b");
        let reduced = reduce_path(path, Some(OsStr::new("/shim")));
        let entries: Vec<PathBuf> = env::split_paths(&reduced).collect();
        assert_eq!(entries, vec![PathBuf::from("/a"), PathBuf::from("/b")]);
    }

    #[test]
    fn reduce_path_leaves_other_entries_untouched_when_shim_dir_absent_from_path() {
        let path = OsStr::new("/a:/b");
        assert_eq!(
            reduce_path(path, Some(OsStr::new("/nowhere"))),
            OsString::from("/a:/b")
        );
    }

    #[test]
    #[cfg(unix)]
    fn empty_path_entries_do_not_panic() {
        let path = OsStr::new("::/usr/bin");
        let reduced = reduce_path(path, Some(OsStr::new("/shim")));
        // Must not panic; the empty (cwd) entry and /usr/bin survive.
        let _ = resolve_program(
            OsStr::new("brief-shim-test-definitely-not-a-real-binary-xyz"),
            &reduced,
        );
    }

    #[test]
    #[cfg(unix)]
    fn non_utf8_path_entries_do_not_panic() {
        use std::os::unix::ffi::OsStrExt;
        let mut bytes = b"/a:".to_vec();
        bytes.extend_from_slice(&[0xff, 0xfe]);
        bytes.extend_from_slice(b":/b");
        let path = OsStr::from_bytes(&bytes);
        let reduced = reduce_path(path, Some(OsStr::new("/shim")));
        let _ = resolve_program(OsStr::new("brief-shim-test-also-not-real-xyz"), &reduced);
    }

    #[test]
    #[cfg(unix)]
    fn resolve_program_finds_a_binary_in_the_reduced_path_never_in_the_shim_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let shim_dir = tmp.path().join("shims");
        let real_dir = tmp.path().join("real");
        std::fs::create_dir_all(&shim_dir).unwrap();
        std::fs::create_dir_all(&real_dir).unwrap();
        make_exec(&shim_dir, "grep");
        let real_grep = make_exec(&real_dir, "grep");

        let path = env::join_paths([&shim_dir, &real_dir]).unwrap();
        let reduced = reduce_path(&path, Some(shim_dir.as_os_str()));
        let resolved = resolve_program(OsStr::new("grep"), &reduced);
        assert_eq!(resolved, Some(real_grep));
    }

    #[test]
    fn resolve_program_bypasses_resolution_for_absolute_or_relative_paths() {
        let path = OsStr::new("/anything");
        assert_eq!(resolve_program(OsStr::new("/usr/bin/grep"), path), None);
        assert_eq!(resolve_program(OsStr::new("./grep"), path), None);
    }

    #[test]
    #[cfg(unix)]
    fn resolve_program_returns_none_when_not_found_anywhere() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().as_os_str();
        assert_eq!(
            resolve_program(OsStr::new("brief-shim-test-nope-xyz"), path),
            None
        );
    }

    #[test]
    fn resolve_spawn_is_unchanged_when_shim_dir_unset() {
        let (program, path) = resolve_spawn(OsStr::new("grep"), None, Some(OsStr::new("/a:/b")));
        assert_eq!(program, OsString::from("grep"));
        assert_eq!(path, None);
    }

    #[test]
    fn resolve_spawn_is_unchanged_when_path_unset() {
        let (program, path) = resolve_spawn(OsStr::new("grep"), Some(OsStr::new("/shim")), None);
        assert_eq!(program, OsString::from("grep"));
        assert_eq!(path, None);
    }

    #[test]
    #[cfg(unix)]
    fn resolve_spawn_resolves_and_reduces_when_both_set() {
        let tmp = tempfile::tempdir().unwrap();
        let shim_dir = tmp.path().join("shims");
        let real_dir = tmp.path().join("real");
        std::fs::create_dir_all(&shim_dir).unwrap();
        std::fs::create_dir_all(&real_dir).unwrap();
        make_exec(&shim_dir, "grep");
        let real_grep = make_exec(&real_dir, "grep");
        let path = env::join_paths([&shim_dir, &real_dir]).unwrap();

        let (program, reduced) = resolve_spawn(
            OsStr::new("grep"),
            Some(shim_dir.as_os_str()),
            Some(path.as_os_str()),
        );
        assert_eq!(program, real_grep.into_os_string());
        let reduced_entries: Vec<PathBuf> = env::split_paths(&reduced.unwrap()).collect();
        assert_eq!(reduced_entries, vec![real_dir]);
    }

    #[test]
    #[cfg(unix)]
    fn resolve_spawn_falls_back_to_original_program_when_unresolvable_but_still_reduces_path() {
        let tmp = tempfile::tempdir().unwrap();
        let shim_dir = tmp.path().join("shims");
        std::fs::create_dir_all(&shim_dir).unwrap();
        let path = shim_dir.as_os_str();

        let (program, reduced) = resolve_spawn(
            OsStr::new("brief-shim-test-missing-xyz"),
            Some(shim_dir.as_os_str()),
            Some(path),
        );
        assert_eq!(program, OsString::from("brief-shim-test-missing-xyz"));
        assert_eq!(reduced, Some(OsString::new()));
    }
}
