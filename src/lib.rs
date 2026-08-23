//! brief: gate large command output behind a token threshold, writing the
//! full output to disk and returning a compact head/tail fold. Below the
//! threshold, output passes through byte-for-byte untouched — nothing is
//! ever destroyed.

mod cli;
mod fold;
mod hook;
mod init;
mod private_fs;
mod report;
mod runner;
mod text_offset;
mod thousands;
mod track;

pub use cli::main_with;
pub use fold::{Fold, FoldConfig, FoldOutcome, fold_output};
pub use runner::{RunOutcome, run, status_to_exit_code};
pub use track::TrackConfig;
