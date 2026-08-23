//! `brief report`: turn the tracking JSONL into a falsifiable savings
//! report. Wiring only — see `cli::run` for the entry point.
//!
//! # Scope limits (restated in every report's own output, not just here)
//!
//! - Only `grep`/`cat`/`find`/`rg` invocations are tracked (`crate::track`'s
//!   documented scope), so every byte total below is "output brief
//!   handled," never "total output" or "your token usage."
//! - `reads_fold` only catches a re-read that goes back through brief's
//!   own argv. A re-read that bypasses brief entirely (a plain shell
//!   `cat` of a fold file, run outside brief) is invisible here, so the
//!   re-read cost this report prints is a lower bound, never a total.

mod aggregate;
mod caveats;
mod cli;
mod load;
mod parse;
mod render_json;
mod render_text;

pub(crate) use cli::run;
