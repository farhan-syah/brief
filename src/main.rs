//! brief CLI entry point. Logic lives in the library crate (`src/cli/`)
//! for testability; this binary stays thin.

use std::io;

fn main() {
    let stdout = io::stdout();
    let stderr = io::stderr();
    let code = brief::main_with(std::env::args_os(), &mut stdout.lock(), &mut stderr.lock());
    // A bare `return` from `main` always exits 0, which would silently
    // swallow the child's real exit status.
    std::process::exit(code);
}
