//! CLI argv dispatch for `brief <program> [args...]`. Wiring only — the
//! dispatch logic lives in `dispatch`, brief's own help/version text in
//! `help`, and the non-target spawn path in `passthrough`.

mod dispatch;
mod help;
mod passthrough;

pub use dispatch::main_with;
// Only reached from `hook::decide`'s cross-module regression test today.
#[cfg(test)]
pub(crate) use dispatch::is_fold_target;
