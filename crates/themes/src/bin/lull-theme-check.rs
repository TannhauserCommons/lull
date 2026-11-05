//! Checks theme packages as the hub's import does: `lull-theme-check <.lulltheme or folder>...`. Prints one line a
//! package, and exits 1 when any would be refused.

use std::io::Write;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut out = std::io::stdout().lock();
    let mut refused = false;
    for path in std::env::args_os().skip(1) {
        let path = std::path::PathBuf::from(path);
        let line = match lull_themes::check(&path) {
            Ok(id) => format!("ok       {}: {id}", path.display()),
            Err(e) => {
                refused = true;
                format!("refused  {}: {e}", path.display())
            }
        };
        let _ = writeln!(out, "{line}");
    }
    if refused { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}
