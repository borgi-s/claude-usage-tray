//! Opt-in "start at Windows login" via the per-user registry Run key
//! (`HKCU\Software\Microsoft\Windows\CurrentVersion\Run`). The registry is the
//! single source of truth: nothing here is cached in settings or shared state.

use std::path::Path;

use anyhow::Context;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY,
    HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE, REG_SAM_FLAGS, REG_SZ,
};

/// The Run-key value name we own. Anthropic-distinct so we never collide with
/// other apps' autostart entries.
const VALUE_NAME: &str = "ClaudeUsageTray";

/// The Run subkey under HKEY_CURRENT_USER. Backslashes are escaped for Rust.
const RUN_SUBKEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";

/// Encode a Rust string as a null-terminated UTF-16 buffer for the Win32 `W` APIs.
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Owns an opened registry key and closes it on drop (incl. `?` early-returns).
struct KeyGuard(HKEY);

impl Drop for KeyGuard {
    fn drop(&mut self) {
        // SAFETY: self.0 came from a successful RegOpenKeyExW and is closed once.
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

/// Open the per-user Run key with the requested access rights.
fn open_run_key(access: REG_SAM_FLAGS) -> anyhow::Result<KeyGuard> {
    let sub = wide(RUN_SUBKEY);
    let mut hkey = HKEY::default();
    // SAFETY: HKEY_CURRENT_USER is a fixed predefined handle; `sub` is a valid
    // null-terminated buffer alive for the call; hkey is written on success.
    let rc = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(sub.as_ptr()),
            0,
            access,
            &mut hkey,
        )
    };
    if rc != ERROR_SUCCESS {
        anyhow::bail!("RegOpenKeyExW(Run) failed: {:?}", rc);
    }
    Ok(KeyGuard(hkey))
}

/// Build the Run-key value string for a given executable path: the absolute
/// path wrapped in double quotes so a path with spaces stays one argument.
fn desired_value(exe: &Path) -> String {
    format!("\"{}\"", exe.display())
}

/// Whether our Run value is currently present. Any error (key/value missing,
/// query failure) is treated as "not enabled".
pub fn is_enabled() -> bool {
    let key = match open_run_key(KEY_READ) {
        Ok(k) => k,
        Err(_) => return false,
    };
    let name = wide(VALUE_NAME);
    // Query existence only: all output pointers are None, so we just read the
    // return code. ERROR_SUCCESS => the value exists.
    // SAFETY: key.0 is open; `name` is a valid buffer alive for the call.
    let rc = unsafe { RegQueryValueExW(key.0, PCWSTR(name.as_ptr()), None, None, None, None) };
    rc == ERROR_SUCCESS
}

/// Register the current executable to launch at login (no arguments => default
/// tray mode). Overwrites any existing value (e.g. a stale path).
pub fn enable() -> anyhow::Result<()> {
    let exe = std::env::current_exe().context("resolving current exe path")?;
    let value = wide(&desired_value(&exe));
    // Reinterpret the u16 buffer as bytes for REG_SZ (incl. its trailing NUL).
    // SAFETY: `value` is alive for the call; byte length is exactly 2 * u16 len.
    let bytes: &[u8] =
        unsafe { std::slice::from_raw_parts(value.as_ptr() as *const u8, value.len() * 2) };

    let key = open_run_key(KEY_SET_VALUE)?;
    let name = wide(VALUE_NAME);
    // SAFETY: key.0 is open with write access; `name` and `bytes` are alive.
    let rc = unsafe { RegSetValueExW(key.0, PCWSTR(name.as_ptr()), 0, REG_SZ, Some(bytes)) };
    if rc != ERROR_SUCCESS {
        anyhow::bail!("RegSetValueExW failed: {:?}", rc);
    }
    Ok(())
}

/// Remove our Run value. A value that is already absent counts as success.
pub fn disable() -> anyhow::Result<()> {
    let key = open_run_key(KEY_SET_VALUE)?;
    let name = wide(VALUE_NAME);
    // SAFETY: key.0 is open with write access; `name` is alive for the call.
    let rc = unsafe { RegDeleteValueW(key.0, PCWSTR(name.as_ptr())) };
    if rc == ERROR_SUCCESS || rc == ERROR_FILE_NOT_FOUND {
        Ok(())
    } else {
        anyhow::bail!("RegDeleteValueW failed: {:?}", rc);
    }
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
