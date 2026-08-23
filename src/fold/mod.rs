//! Size-gated output folding: below `threshold_tokens` output passes
//! through untouched; above it, the full output is written to a private,
//! rotating file on disk and a compact head/tail summary is returned.
//! Nothing is ever destroyed — the persisted file is never truncated.
//!
//! Disk-write machinery (slug sanitization, private dir/file creation,
//! rotation, path display/quoting) is ported from rtk's `core::tee` and
//! `core::utils` — see the provenance comment at the top of each file.

// `pub(crate)` where the streaming runner (`crate::runner`) reuses the same
// machinery the in-memory path uses — one fold-file writer, one rotation
// rule, one token gate, one `Fold` constructor.
mod config;
pub(crate) mod paths;
pub(crate) mod rotate;
mod slug;
pub(crate) mod summary;
pub(crate) mod tokens;
pub(crate) mod write;

pub use config::FoldConfig;
pub use summary::{Fold, FoldOutcome, fold_output};
