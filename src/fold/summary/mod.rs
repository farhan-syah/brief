//! Fold construction and summary preview helpers.

mod fold;
mod lines;
mod preview;

pub(crate) use fold::TAIL_LINES;
pub use fold::{Fold, FoldOutcome, fold_output};
pub(crate) use lines::total_lines_from;
pub(crate) use preview::SLICE_MAX_BYTES;
