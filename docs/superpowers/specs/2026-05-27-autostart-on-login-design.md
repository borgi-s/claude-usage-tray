# Auto-start on login — design

**Date:** 2026-05-27
**Status:** approved, pre-implementation
**Target version:** `v1.1.0` (post-1.0 polish — minor bump)

## Goal

Let the user opt into having the tray widget launch automatically when they log
into Windows. Off by default; never enabled without an explicit user action.

## Decisions (locked in brainstorming)

- **Mechanism:** Registry Run key —
  `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`. No admin rights,
  per-user, trivially toggled. Uses the `windows` crate (already a dependency);
  no new crate.
- **Control surface:** both the Settings tab (checkbox) and the tray right-click
  menu (checkable item).
- **Source of truth:** the registry itself. Nothing is cached in `settings.toml`
  or in shared state. Each UI surface reads the live registry value when it
  renders, so the two surfaces stay consistent for free.

## New module: `src/autostart.rs`

Owns all registry interaction. The Run value is named `ClaudeUsageTray` and
holds the quoted absolute path to the current executable
(`std::env::current_exe()`), with no arguments — launching in the default tray
mode.

Public API:

- `pub fn is_enabled() -> bool`
  Open the Run key (`RegOpenKeyExW`, read access), query our value
  (`RegQueryValueExW`), return whether it is present. Any error (key missing,
  value missing, query failure) → `false`.
- `pub fn enable() -> anyhow::Result<()>`
  Open/create the Run key with write access, set our value (`RegSetValueExW`,
  `REG_SZ`) to the quoted current-exe path.
- `pub fn disable() -> anyhow::Result<()>`
  Delete our value (`RegDeleteValueW`). Treat "value already absent" as success.

Registry strings are UTF-16 wide (`REG_SZ`), consistent with the existing Win32
code under `src/tray/`. `RegCloseKey` is called on every path (an RAII guard
holding the `HKEY` is the clean way to guarantee this on early returns).

### Pure, testable helpers

Factor the non-side-effecting logic out so it can be unit-tested without touching
the real registry:

- `const VALUE_NAME: &str = "ClaudeUsageTray";`
- `fn desired_value(exe: &Path) -> String` — produces the quoted path string
  (e.g. `"C:\\path\\to\\claude-usage-tray.exe"`), wrapping in double quotes so a
  path containing spaces parses as a single token.

## UI integration

### Settings tab (`src/dashboard/settings_tab.rs`)

A `Start on login` checkbox in its own row, **outside** the draft/Save flow used
by the other settings. Rationale: with the registry as truth, the toggle must
apply immediately and reflect external reality, not sit in a pending draft.

- Each frame, read `autostart::is_enabled()` to drive the checkbox's checked
  state.
- On click, call `enable()` / `disable()` immediately.
- On error, show an inline red message next to the checkbox (mirrors the
  existing save-error styling).

This row does not affect the `dirty` flag or the `Save` button.

### Tray menu (`src/tray/window.rs`)

A checkable `Start on login` item, new command id `IDM_AUTOSTART`, inserted
before the Quit separator. The popup menu is rebuilt on every right-click, so:

- At build time, query `is_enabled()` and append the item with `MF_CHECKED` or
  `MF_UNCHECKED`.
- In the `WM_COMMAND` handler, branch on `IDM_AUTOSTART`: read current state,
  toggle it via `enable()`/`disable()`, and log any error via `tracing` (no
  modal dialog).

No shared state between the two surfaces — both read the registry directly.

## Edge cases & error handling

- **Exe moved after enabling:** the stored path goes stale and auto-start
  silently fails to launch. v1 deliberately does **not** self-heal (YAGNI);
  re-toggling the checkbox rewrites the path. Documented as an accepted cut.
- **Write / permission failure:** surfaced inline in Settings, logged from the
  tray path. HKCU writes do not require admin, so this should be rare.
- **Default state:** off. We never write the Run value without an explicit user
  toggle.

## Testing

- Unit tests for the pure helpers: `desired_value` quoting (incl. a path with
  spaces) and the `VALUE_NAME` constant.
- The real `enable` / `disable` / `is_enabled` round-trip mutates the live
  per-user registry, so it is verified manually in the GUI — the same approach
  used to verify settings live-apply for the `v1.0.0` milestone. Manual check:
  toggle in Settings → confirm value appears under the Run key; toggle off →
  confirm it's gone; confirm the tray menu checkmark reflects each change.

## Out of scope

- Self-healing a stale exe path (see edge cases).
- A startup delay or "run with highest privileges" (those would require Task
  Scheduler, explicitly rejected).
- Any `settings.toml` schema change.
