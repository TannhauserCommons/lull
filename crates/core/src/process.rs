//! Starting another program. On a Windows desktop hub (the app with no console of its own), a
//! console program started plainly opens a console window, and closing that window kills the
//! program. Every launch goes through [`command`], which keeps the program windowless; clippy's
//! `disallowed-methods` (clippy.toml) refuses `Command::new` anywhere else.

use std::ffi::OsStr;
use std::process::Command;

/// A command for `program` that never opens a console window. For tokio,
/// `tokio::process::Command::from(command(..))` keeps that.
#[allow(clippy::disallowed_methods)] // the one place a Command is made
pub fn command(program: impl AsRef<OsStr>) -> Command {
    let mut cmd = Command::new(program);
    no_window(&mut cmd);
    cmd
}

#[cfg(windows)]
fn no_window(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    /// Win32's CREATE_NO_WINDOW: a console program runs with a console but no window.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

/// Elsewhere a program started without a terminal gets none.
#[cfg(not(windows))]
fn no_window(_: &mut Command) {}

#[cfg(all(test, windows))]
mod tests {
    use super::command;
    use std::os::windows::process::CommandExt;

    /// Set in the copy of this test that plays the desktop app.
    const AS_APP: &str = "LULL_PROCESS_TEST_AS_APP";
    /// The line that copy prints: the started program's console window, 0 for none.
    const SAID: &str = "started program's console window: ";

    /// A console program started through `command` by a process without a console, as the
    /// desktop app is, gets no console window. The test runs itself again detached from its
    /// console to be that app (a program started by one with a console would share its window).
    #[test]
    fn a_console_program_started_by_the_app_has_no_window() -> Result<(), String> {
        if std::env::var_os(AS_APP).is_some() {
            println!("{SAID}{}", console_window_of_started_program()?);
            return Ok(());
        }
        /// Win32's DETACHED_PROCESS: the copy runs with no console at all.
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let out = command(exe)
            .args([
                "--exact",
                "process::tests::a_console_program_started_by_the_app_has_no_window",
                "--nocapture",
            ])
            .env(AS_APP, "1")
            .creation_flags(DETACHED_PROCESS)
            .output()
            .map_err(|e| format!("the test as the app: {e}"))?;
        let said = String::from_utf8_lossy(&out.stdout);
        match said.lines().find_map(|l| l.strip_prefix(SAID)) {
            Some("0") => Ok(()),
            Some(window) => Err(format!("the started program opened console window {window}")),
            None => Err(format!(
                "the test as the app said: {said}{}",
                String::from_utf8_lossy(&out.stderr)
            )),
        }
    }

    /// Starts PowerShell, which every Windows has, and asks it for its own console window.
    fn console_window_of_started_program() -> Result<String, String> {
        let ask = "(Add-Type -Name W -Namespace L -PassThru -MemberDefinition \
                   '[DllImport(\"kernel32.dll\")] public static extern IntPtr GetConsoleWindow();')::GetConsoleWindow()";
        let out = command("powershell.exe")
            .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", ask])
            .output()
            .map_err(|e| format!("powershell: {e}"))?;
        if !out.status.success() {
            return Err(format!("powershell failed: {}", String::from_utf8_lossy(&out.stderr)));
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    }
}
