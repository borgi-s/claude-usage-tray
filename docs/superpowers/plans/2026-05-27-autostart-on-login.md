# Auto-start on Login Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let the user opt into launching the tray widget at Windows login, toggled from both the Settings tab and the tray menu, with the registry as the source of truth.

**Architecture:** A new `src/autostart.rs` module owns all registry I/O against `HKCU\…\CurrentVersion\Run` using the `windows` crate. Both UI surfaces read the live registry value when they render and call `enable()`/`disable()` on toggle — no cached state, no `settings.toml` change.

**Tech Stack:** Rust, `windows` 0.58 crate (`Win32_System_Registry`), `eframe`/`egui` 0.29, Win32 menus.

**Spec:** `docs/superpowers/specs/2026-05-27-autostart-on-login-design.md`

**Beginner note (this whole plan):** The user is a Rust beginner. Steps below explain the unfamiliar idioms inline — registry FFI, RAII drop guards, `unsafe` slice reinterpretation, and `PCWSTR` wide-string pointers. The existing `src/tray/window.rs` is the local reference for the Win32 patterns.

**Windows worktree gotcha:** Build/test in the main checkout, not a nested `.claude/worktrees/...` path (MAX_PATH link failures). If linking OOMs, use `cargo test -j 1`.

---

### Task 1: Module skeleton + pure helper (TDD)

**Files:**
- Modify: `Cargo.toml:30-38` (add registry feature)
- Modify: `src/lib.rs:1-17` (declare module)
- Create: `src/autostart.rs`

- [ ] **Step 1: Add the registry feature to the `windows` dependency**

In `Cargo.toml`, the `windows` features list (currently ending at `"Win32_UI_WindowsAndMessaging",`) gains one entry. The full block becomes:

```toml
windows = { version = "0.58", features = [
    "Win32_Foundation",
    "Win32_Graphics_Gdi",
    "Win32_Graphics_GdiPlus",
    "Win32_System_Console",
    "Win32_System_LibraryLoader",
    "Win32_System_Registry",
    "Win32_UI_Shell",
    "Win32_UI_WindowsAndMessaging",
] }
```

- [ ] **Step 2: Declare the module in `src/lib.rs`**

Add the line in alphabetical position (after `pub mod api;`):

```rust
pub mod autostart;
```

- [ ] **Step 3: Write the failing test for `desired_value`**

Create `src/autostart.rs` with ONLY the pure helper and its test (the registry functions come in Task 2). `desired_value` wraps an exe path in double quotes so a path containing spaces is parsed by the shell as a single token.

```rust
//! Opt-in "start at Windows login" via the per-user registry Run key
//! (`HKCU\Software\Microsoft\Windows\CurrentVersion\Run`). The registry is the
//! single source of truth: nothing here is cached in settings or shared state.

use std::path::Path;

/// The Run-key value name we own. Anthropic-distinct so we never collide with
/// other apps' autostart entries.
const VALUE_NAME: &str = "ClaudeUsageTray";

/// The Run subkey under HKEY_CURRENT_USER. Backslashes are escaped for Rust.
const RUN_SUBKEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";

/// Build the Run-key value string for a given executable path: the absolute
/// path wrapped in double quotes so a path with spaces stays one argument.
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
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test --lib autostart`
Expected: PASS (`desired_value_quotes_the_path`). This also confirms the new module compiles and the feature addition didn't break the build.

- [ ] **Step 5: Commit**

```powershell
git add Cargo.toml Cargo.lock src/lib.rs src/autostart.rs
git commit -m "feat(autostart): add module skeleton + path-quoting helper"
```

(Cargo.lock is staged because adding a feature can change it.)

---

### Task 2: Registry enable / disable / is_enabled

**Files:**
- Modify: `src/autostart.rs`

**Beginner notes for this task:**
- **`PCWSTR`** is a pointer to a null-terminated UTF-16 string. We build a `Vec<u16>` (via `wide`) and pass `PCWSTR(v.as_ptr())`. The `Vec` must outlive the call — keep it in a local binding, don't inline it.
- **RAII guard:** `KeyGuard` holds the opened `HKEY` and calls `RegCloseKey` in `Drop`, so the key is closed on every return path (including `?` early-returns) — the same pattern as `GdiplusGuard` in `main.rs`.
- **`unsafe` slice:** `RegSetValueExW` wants the value as raw bytes. A `&[u16]` and a `&[u8]` of twice the length point at the same memory; `from_raw_parts` reinterprets it. Safe here because the `Vec<u16>` is alive and we compute the length exactly.
- **Error type:** `Reg*` functions return `WIN32_ERROR`; `ERROR_SUCCESS` (value 0) means OK. We compare directly.

- [ ] **Step 1: Add imports at the top of `src/autostart.rs`**

Below the existing `use std::path::Path;`:

```rust
use anyhow::Context;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY,
    HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE, REG_SAM_FLAGS, REG_SZ,
};
```

- [ ] **Step 2: Add the wide-string helper and the RAII key guard**

Add after the `RUN_SUBKEY` const:

```rust
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
```

- [ ] **Step 3: Add `is_enabled`, `enable`, `disable`**

```rust
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
    let rc =
        unsafe { RegSetValueExW(key.0, PCWSTR(name.as_ptr()), 0, REG_SZ, Some(bytes)) };
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
```

- [ ] **Step 4: Verify it compiles clean (no new automated test)**

These three functions mutate the real per-user registry, so they are verified manually in Task 5, not unit-tested. Confirm the build is clean now:

Run: `cargo build` then `cargo clippy --all-targets -- -D warnings`
Expected: both succeed with no warnings.

- [ ] **Step 5: Commit**

```powershell
git add src/autostart.rs
git commit -m "feat(autostart): registry enable/disable/is_enabled via HKCU Run key"
```

---

### Task 3: Settings tab checkbox

**Files:**
- Modify: `src/dashboard/app.rs:69`, `src/dashboard/app.rs:95`, `src/dashboard/app.rs:292-297`
- Modify: `src/dashboard/settings_tab.rs:34-39` (signature), and insert a row before `app.rs:108`'s separator

**Design note:** This checkbox is intentionally OUTSIDE the draft/Save flow — it reads `is_enabled()` live each frame and applies on click, matching the registry-as-truth model.

- [ ] **Step 1: Add the error-state field to `DashboardApp`**

In `src/dashboard/app.rs`, after the `settings_save_msg` field (line 69):

```rust
    settings_save_msg: Option<Result<(), String>>,
    autostart_msg: Option<Result<(), String>>,
}
```

- [ ] **Step 2: Initialize the new field**

In `DashboardApp::new`, after `settings_save_msg: None,` (line 95):

```rust
            settings_save_msg: None,
            autostart_msg: None,
        }
```

- [ ] **Step 3: Pass it into the settings tab render call**

Replace the `Tab::Settings` arm (lines 292-297):

```rust
                crate::dashboard::settings_tab::render(
                    ui,
                    &mut self.settings_draft,
                    &self.settings,
                    &mut self.settings_save_msg,
                    &mut self.autostart_msg,
                );
```

- [ ] **Step 4: Extend the `render` signature in `settings_tab.rs`**

Add the new parameter (after `save_msg`, line 38):

```rust
pub fn render(
    ui: &mut Ui,
    draft: &mut Settings,
    shared: &SharedSettings,
    save_msg: &mut Option<Result<(), String>>,
    autostart_msg: &mut Option<Result<(), String>>,
) {
```

- [ ] **Step 5: Insert the auto-start row before the bottom separator**

In `settings_tab.rs`, the block at line 108 currently reads:

```rust
    ui.add_space(16.0);
    ui.separator();
    ui.add_space(8.0);
```

Insert the auto-start row immediately BEFORE it:

```rust
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        // Reads the live registry value each frame; applies immediately on toggle.
        let mut enabled = crate::autostart::is_enabled();
        if ui.checkbox(&mut enabled, "Start on login").changed() {
            let res = if enabled {
                crate::autostart::enable()
            } else {
                crate::autostart::disable()
            };
            *autostart_msg = Some(res.map_err(|e| e.to_string()));
        }
        if let Some(Err(e)) = autostart_msg.as_ref() {
            ui.label(
                RichText::new(format!("✗ {e}")).color(egui::Color32::from_rgb(220, 120, 120)),
            );
        }
    });

    ui.add_space(16.0);
    ui.separator();
    ui.add_space(8.0);
```

(`RichText` and `egui` are already imported at the top of the file.)

- [ ] **Step 6: Verify it compiles clean**

Run: `cargo build` then `cargo clippy --all-targets -- -D warnings`
Expected: both succeed, no warnings. (GUI behavior is verified in Task 5.)

- [ ] **Step 7: Commit**

```powershell
git add src/dashboard/app.rs src/dashboard/settings_tab.rs
git commit -m "feat(autostart): Start-on-login checkbox in Settings tab"
```

---

### Task 4: Tray menu item

**Files:**
- Modify: `src/tray/window.rs` — imports (~line 19-24), `IDM_*` consts (~line 35-39), `WM_COMMAND` arm (~line 204-206), menu build (~line 503-516)

**Beginner note:** Win32 menu flags (`MF_STRING`, `MF_CHECKED`, …) are bitflags; combine with `|`. The popup menu is rebuilt on every right-click, so reading `is_enabled()` at build time keeps the checkmark honest.

- [ ] **Step 1: Add `MF_CHECKED` and `MF_UNCHECKED` to the WindowsAndMessaging import**

In the `use windows::Win32::UI::WindowsAndMessaging::{…}` block, the line containing `MF_STRING` (line 23) gains the two flags. It currently includes `MF_STRING, MSG, …`; change to include the menu flags alongside the existing `MF_SEPARATOR`/`MF_STRING`:

```rust
    MF_CHECKED, MF_SEPARATOR, MF_STRING, MF_UNCHECKED, MSG, SW_SHOWNORMAL, TPM_LEFTBUTTON,
```

(Keep the rest of that import line unchanged; just ensure `MF_CHECKED` and `MF_UNCHECKED` are present. `MF_SEPARATOR`/`MF_STRING` are already imported — don't duplicate.)

- [ ] **Step 2: Add the `IDM_AUTOSTART` command id**

After the `IDM_CHECK_UPDATES` const (line 39):

```rust
pub const IDM_CHECK_UPDATES: usize = 3;

/// Tray menu command id: toggle "start at login".
pub const IDM_AUTOSTART: usize = 4;
```

- [ ] **Step 3: Handle the new command in `WM_COMMAND`**

In the `match wparam.0 & 0xFFFF` block, after the `IDM_CHECK_UPDATES` arm (lines 204-206), add:

```rust
                id if id == IDM_CHECK_UPDATES => {
                    with_state(hwnd, |state| trigger_manual_check(hwnd, state));
                }
                id if id == IDM_AUTOSTART => {
                    let res = if crate::autostart::is_enabled() {
                        crate::autostart::disable()
                    } else {
                        crate::autostart::enable()
                    };
                    if let Err(e) = res {
                        tracing::warn!(error = %e, "failed to toggle auto-start");
                    }
                }
```

- [ ] **Step 4: Append the checkable item to the popup menu**

In the menu-build `unsafe` block (lines 505-516), add the auto-start item just before the `IDM_QUIT` line. First add its label next to `check_label`/`quit_label` (line 503-504):

```rust
    let check_label = encode_utf16("Check for updates now");
    let autostart_label = encode_utf16("Start on login");
    let quit_label = encode_utf16("Quit");
```

Then inside the `unsafe` block, after the `IDM_CHECK_UPDATES` append and before the `IDM_QUIT` append:

```rust
        let _ = AppendMenuW(
            hmenu,
            MF_STRING,
            IDM_CHECK_UPDATES,
            PCWSTR(check_label.as_ptr()),
        );
        let autostart_flags = MF_STRING
            | if crate::autostart::is_enabled() {
                MF_CHECKED
            } else {
                MF_UNCHECKED
            };
        let _ = AppendMenuW(
            hmenu,
            autostart_flags,
            IDM_AUTOSTART,
            PCWSTR(autostart_label.as_ptr()),
        );
        let _ = AppendMenuW(hmenu, MF_STRING, IDM_QUIT, PCWSTR(quit_label.as_ptr()));
```

- [ ] **Step 5: Verify it compiles clean**

Run: `cargo build` then `cargo clippy --all-targets -- -D warnings`
Expected: both succeed, no warnings.

- [ ] **Step 6: Commit**

```powershell
git add src/tray/window.rs
git commit -m "feat(autostart): checkable Start-on-login item in tray menu"
```

---

### Task 5: Manual GUI verification, version bump, docs, tag

**Files:**
- Modify: `Cargo.toml:3` (version), `Cargo.lock` (regenerated)
- Modify: `CLAUDE.md` (roadmap / milestone note)

- [ ] **Step 1: Full verification build**

Run: `cargo fmt` then `cargo clippy --all-targets -- -D warnings` then `cargo test`
Expected: fmt makes no/idempotent changes; clippy clean; all tests pass (incl. `desired_value_quotes_the_path`).

- [ ] **Step 2: Manual GUI verification (registry round-trip)**

Build and run the tray app: `cargo run` (launches in tray mode). Then verify each of:
  1. Open the dashboard → Settings tab. Tick **Start on login**.
  2. Confirm the value exists: `Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -Name ClaudeUsageTray` shows the quoted exe path.
  3. Right-click the tray icon → confirm **Start on login** shows a checkmark.
  4. Untick it in Settings (or click the tray item). Confirm the registry value is gone: the same `Get-ItemProperty` now errors with "Property ClaudeUsageTray does not exist".
  5. Toggle once more from the tray menu and confirm the Settings checkbox reflects it when reopened.

Expected: registry value appears/disappears in lockstep with both controls; the tray checkmark and Settings checkbox always agree.

- [ ] **Step 3: Bump the version**

In `Cargo.toml`, line 3:

```toml
version = "1.1.0"
```

- [ ] **Step 4: Regenerate Cargo.lock and update CLAUDE.md**

Run `cargo build` so `Cargo.lock`'s own `version =` line updates. Then add a roadmap note in `CLAUDE.md` under "Active design + plans" (a `v1.1.0` post-1.0 entry referencing this spec + plan and noting auto-start shipped).

- [ ] **Step 5: Commit and tag**

```powershell
git add Cargo.toml Cargo.lock CLAUDE.md
git commit -m "chore: bump to v1.1.0 (auto-start on login)"
git tag v1.1.0
```

(Push is left to the user, consistent with prior releases.)

---

## Self-Review

**Spec coverage:**
- New `autostart.rs` module, registry Run key, value name `ClaudeUsageTray`, quoted current-exe path → Tasks 1-2. ✓
- `is_enabled`/`enable`/`disable` API, registry-as-truth, no settings.toml field → Task 2. ✓
- Pure testable helper (`desired_value`, `VALUE_NAME`) → Task 1. ✓
- Settings tab checkbox, immediate-apply, outside draft/Save, inline error → Task 3. ✓
- Tray menu checkable item, rebuilt-per-click checkmark, `tracing` on error → Task 4. ✓
- Default off (never written without explicit toggle) → inherent: no code writes the value except `enable()`. ✓
- Self-heal deliberately omitted → not implemented (matches the spec's accepted cut). ✓
- Manual GUI verification of the registry round-trip → Task 5 Step 2. ✓
- `v1.1.0` minor bump → Task 5. ✓

**Placeholder scan:** No TBD/TODO/"handle edge cases" — all steps contain concrete code or exact commands.

**Type consistency:** `is_enabled() -> bool`, `enable()/disable() -> anyhow::Result<()>`, `desired_value(&Path) -> String`, `VALUE_NAME`/`RUN_SUBKEY` consts, `IDM_AUTOSTART: usize`, `autostart_msg: Option<Result<(), String>>` — names used identically across Tasks 1-5.
