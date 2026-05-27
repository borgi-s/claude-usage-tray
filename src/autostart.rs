//! Opt-in "start at Windows login" via the per-user registry Run key
//! (`HKCU\Software\Microsoft\Windows\CurrentVersion\Run`). The registry is the
//! single source of truth: nothing here is cached in settings or shared state.

use std::path::Path;

/// The Run-key value name we own. Anthropic-distinct so we never collide with
/// other apps' autostart entries.
#[allow(dead_code)]
const VALUE_NAME: &str = "ClaudeUsageTray";

/// The Run subkey under HKEY_CURRENT_USER. Backslashes are escaped for Rust.
#[allow(dead_code)]
const RUN_SUBKEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";

/// Build the Run-key value string for a given executable path: the absolute
/// path wrapped in double quotes so a path with spaces stays one argument.
#[allow(dead_code)]
fn desired_value(exe: &Path) -> String {
    format!("\"{}\"", exe.display())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn desired_value_quotes_the_path() {
        let v = desired_value(Path::new("C:\\Program Files\\claude-usage-tray.exe"));
        assert_eq!(v, "\"C:\\Program Files\\claude-usage-tray.exe\"");
    }
}
