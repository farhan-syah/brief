//! Run a command with each of its output streams behind the fold gate.
//!
//! stdout and stderr are fully independent: separate state machine, separate
//! threshold evaluation, separate fold file, separate destination fd. They
//! are never merged or interleaved. Below the gate a stream's raw bytes are
//! written to the real fd untouched — never through a `String`, never a lossy
//! decode, because child output is arbitrary bytes and may be binary. Above
//! the gate the full output is on disk, untruncated, and only a compact
//! head/tail summary is printed.
//!
//! Folding never depends on whether the destination is a terminal:
//! `brief cmd > file.txt` behaves identically to `brief cmd | cat`.
//! Invoking brief is the opt-in; there is no isatty heuristic.
//!
//! # Consequence: output appears when the child exits
//!
//! A size gate cannot decide before it has seen enough bytes, and it cannot
//! undo a decision once bytes have been printed. So every stream is held
//! until the child exits, then written in one go. Content is byte-identical
//! to running the command directly; only the timing differs. A long-running
//! command shows nothing while it works, and progress output that relies on
//! incremental display (spinners, percent counters) arrives all at once at
//! the end. Memory is still bounded: a stream that crosses the gate spills to
//! its fold file and stops accumulating.
//!
//! Nothing is a pty — pipes only. The isatty/colour difference that gives is
//! inherent to any pipe wrapper and out of scope here.
//!
//! `run`/`run_with` also append one best-effort tracking row per invocation
//! (`crate::track`) — an internal side effect, not part of the fold
//! contract above.

mod exit;
mod spawn;
mod spill;

pub use exit::status_to_exit_code;
pub use spawn::{RunOutcome, run};
// `pub(crate)` re-exports: the CLI dispatch (`crate::cli`) drives a child
// through the same fold-gated path the public `run` uses, with explicit
// `out`/`err` destinations, and needs the same basename logic `spawn`
// already uses to slug a fold file.
pub(crate) use spawn::{args_read_fold_dir, basename, cmd_args_and_cwd, run_with};
