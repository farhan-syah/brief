//! CLI argv dispatch for `ogt <program> [args...]`. Wiring only — the
//! dispatch logic lives in `dispatch`, ogt's own help/version text in
//! `help`, and the non-target spawn path in `passthrough`.

mod dispatch;
mod help;
mod passthrough;
mod path_shim;

pub use dispatch::main_with;
// Only reached from `hook::decide`'s cross-module regression test today.
#[cfg(test)]
pub(crate) use dispatch::is_fold_target;
// Shared with `init::shims::render_shim` so the exported env var name in a
// generated shim script and the one `dispatch::main_with` reads can never
// drift apart — see `path_shim`'s doc comment on the constant itself.
pub(crate) use path_shim::OGT_SHIM_DIR;
