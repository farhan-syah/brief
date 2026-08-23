//! sigfold: gate large command output behind a token threshold, writing the
//! full output to disk and returning a compact head/tail fold. Below the
//! threshold, output passes through byte-for-byte untouched — nothing is
//! ever destroyed.

mod fold;

pub use fold::{Fold, FoldConfig, FoldOutcome, fold_output};
