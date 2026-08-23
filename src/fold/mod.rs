//! Size-gated output folding: below `threshold_tokens` output passes
//! through untouched; above it, the full output is written to a private,
//! rotating file on disk and a compact head/tail summary is returned.
//! Nothing is ever destroyed — the persisted file is never truncated.
//!
//! Disk-write machinery (slug sanitization, private dir/file creation,
//! rotation, path display/quoting) is ported from rtk's `core::tee` and
//! `core::utils` — see the provenance comment at the top of each file.

mod config;
mod paths;
mod private_fs;
mod rotate;
mod slug;
mod summary;
mod tokens;
mod write;

pub use config::FoldConfig;
pub use summary::{Fold, FoldOutcome, fold_output};
