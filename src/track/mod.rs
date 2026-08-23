//! Append-only invocation tracking: one audit row per `brief` run,
//! recording raw bytes produced versus bytes that reached the caller for
//! each stream. This is what makes a fold's savings claim falsifiable —
//! without it, the only evidence of a fold's effect is the already-folded
//! output. An internal side effect of running a command, like fold-file
//! rotation — not part of the public folding API.
//!
//! `reads_fold` is the one re-read signal brief can observe honestly:
//! whether this invocation's own argv resolves into the fold directory.
//! A read that bypasses brief entirely (a shell `cat` on a fold file run
//! outside brief) is not observable here, so `reads_fold` undercounts
//! re-reads rather than over-claiming them.
//!
//! Only the fold-target programs (`crate::cli`'s target list) are captured
//! (`captured: true`, with measured byte counts). Everything else runs with
//! fully inherited stdio, which brief never observes and therefore cannot
//! measure — except that a non-target invocation whose own argv reads back
//! a fold file (the `brief tail ...` the fold hint prescribes) still
//! produces an uncaptured row (`captured: false`, no byte counts) so that
//! recovery read is visible in `reads_fold`; see `crate::cli::passthrough`.
//! That makes the sum of `*_raw_bytes` over every *captured* row the
//! denominator for "output brief handled", not "output the machine
//! produced" — a report built on these rows must not claim the latter, and
//! `report::aggregate` must never let an uncaptured row's absent bytes
//! leak into that sum.
//!
//! The tracking file is hard-bounded at `TrackConfig::compact_trigger_bytes`
//! (40 MiB by default — see `retention`) no matter how heavily the tool is
//! used. An audit trail that can grow without limit is one a user
//! eventually deletes, taking the evidence with it.

mod append;
mod config;
mod paths;
mod record;
mod retention;

pub(crate) use append::append;
pub use config::TrackConfig;
pub(crate) use paths::resolve_track_path;
pub(crate) use record::{InvocationRecord, json_string, now_ms};
