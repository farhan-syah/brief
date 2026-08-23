//! Size-gated output folding: below `threshold_tokens` output passes
//! through untouched; above it, the full output is written to a private,
//! rotating file on disk and a compact head/tail summary is returned.
//! Nothing is ever destroyed — the persisted file is never truncated.
//!
//! Disk-write machinery (slug sanitization, private dir/file creation,
//! rotation, path display/quoting) is ported from rtk's `core::tee` and
//! `core::utils` — see the provenance comment at the top of each file.
//!
//! Per-path scoping — folding only under configured project roots — is
//! `scope` (pure: parse the roots file/`BRIEF_ROOTS` text, decide whether
//! a directory is in scope) and `roots` (I/O: locate/read the roots file
//! or env var, canonicalize, and apply the pure decision to the real
//! current directory). `config::FoldConfig::from_env` folds the result
//! into `enabled`, the same field `BRIEF`/`BRIEF_ENABLED` already gate.

// `pub(crate)` where the streaming runner (`crate::runner`) reuses the same
// machinery the in-memory path uses — one fold-file writer, one rotation
// rule, one token gate, one `Fold` constructor.
mod config;
pub(crate) mod paths;
mod roots;
pub(crate) mod rotate;
mod scope;
mod slug;
pub(crate) mod summary;
pub(crate) mod tokens;
pub(crate) mod write;

pub use config::FoldConfig;
pub use summary::{Fold, FoldOutcome, fold_output};
