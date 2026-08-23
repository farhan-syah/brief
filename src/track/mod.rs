//! Append-only invocation tracking: one audit row per `sigfold` run,
//! recording raw bytes produced versus bytes that reached the caller for
//! each stream. This is what makes a fold's savings claim falsifiable —
//! without it, the only evidence of a fold's effect is the already-folded
//! output. An internal side effect of running a command, like fold-file
//! rotation — not part of the public folding API.
//!
//! `reads_fold` is the one re-read signal sigfold can observe honestly:
//! whether this invocation's own argv resolves into the fold directory.
//! A read that bypasses sigfold entirely (a shell `cat` on a fold file run
//! outside sigfold) is not observable here, so `reads_fold` undercounts
//! re-reads rather than over-claiming them.
//!
//! Only the fold-target programs (`crate::cli`'s target list) are captured,
//! so only they produce a row. Everything else runs with fully inherited
//! stdio, which sigfold never observes and therefore cannot measure. That
//! makes the sum of `*_raw_bytes` over every row the denominator for
//! "output sigfold handled", not "output the machine produced" — a report
//! built on these rows must not claim the latter.
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
pub(crate) use record::{InvocationRecord, now_ms};
