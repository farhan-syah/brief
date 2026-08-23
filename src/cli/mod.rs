//! CLI argv dispatch for `brief <program> [args...]`. Wiring only — the
//! dispatch logic lives in `dispatch`, brief's own help/version text in
//! `help`, and the non-target spawn path in `passthrough`.

mod dispatch;
mod help;
mod passthrough;

pub use dispatch::main_with;
