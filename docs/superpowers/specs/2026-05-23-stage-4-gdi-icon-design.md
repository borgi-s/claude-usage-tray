# Stage 4 — GDI+ Rendered Percentage Icon Design

Companion to the top-level [rust-tray-widget-design.md](2026-05-22-rust-tray-widget-design.md). This document fixes the details that the top-level spec left open for Stage 4.

## Goal

Replace Stage 3's four static `IconSet` HICONs with a GDI+-rendered icon that shows the current utilization as a 1–2 character glyph on a color-graded background. Everything else stays unchanged: same threading model, same polling cadence, same tooltip, same right-click menu, same shutdown flow. Only the icon bitmap's contents are new.

This is the "glanceable" stage of the project — the user no longer has to hover to see the number.

## Non-goals (Stage 4)

- ❌ Calibration math against local cache — Stage 5.
- ❌ True DPI awareness / multi-resolution icons — single 16×16 bitmap per render. High-DPI displays will get upsampling blur from the shell. Acceptable for v0.4.0; revisit if it bothers users.
- ❌ Animation or smooth transitions — each redraw shows the current int, no interpolation.
- ❌ Configurable thresholds, fonts, or colors — hardcoded; settings UI deferred.
- ❌ Changes to the tooltip, right-click menu, polling thread, or shutdown sequence (all Stage 3 carries over verbatim).
- ❌ Drawing miniature bars or sparklines inside the icon — number + color only.

## Locked-in design decisions

Settled during the Stage 4 brainstorm (visual mockups on the `design/stage-4-mockups` branch):

| Decision | Value |
|---|---|
| Number displayed | `round(max(five_hour.util, seven_day.util) * 100)` |
| Above 100% | Render `!` instead of digits |
| No data (`Initial` / `RateLimited` / `Error`) | Render `?` on solid gray |
| Color thresholds | `<60` / `60–85` / `≥85` (same as Stage 3) |
| Color mapping | Anchored gradient: green `#2eb82e` @ 0%, yellow `#e6b800` @ 60%, red `#cc2929` @ 85%+ (clamps to red) |
| Text style | White fill + 1-pixel black outline (GDI+ `Pen` width = 1.0 at 16×16 bitmap resolution; two-pass — stroke then fill) |
| Rendering API | GDI+ via the `windows` crate's `Win32_Graphics_GdiPlus` feature |
| Bitmap size | 16×16 ARGB, fixed (no per-DPI adaptation in Stage 4) |
| Font (starting point) | Segoe UI Black at ~10 pt; tune during implementation |

## Icon contents by state

| State | Background | Glyph |
|---|---|---|
| `Initial` | Solid gray `#808080` | `?` |
| `Ok` and `util ≤ 1.00` | Anchored gradient color | 1 or 2 digit number |
| `Ok` and `util > 1.00` | Solid red `#cc2929` | `!` |
| `RateLimited` | Solid gray `#808080` | `?` |
| `Error(_)` | Solid gray `#808080` | `?` |

Note: the cached last-known sample is *not* shown as a stale number when the current poll is RateLimited/Error. This matches Stage 3's "drop to gray on stale" behavior and prevents confusion. The cached value continues to surface in the tooltip's `(stale Nm)` footer.

## Color mapping math

For `Ok` with `util ∈ [0.0, 1.0]`, the background is a linear interpolation between three anchor colors:

```text
util = 0.00 → (46, 184, 46)    #2eb82e  green
util = 0.60 → (230, 184, 0)    #e6b800  yellow
util = 0.85 → (204, 41, 41)    #cc2929  red
util ≥ 0.85 → red (clamped)
```

Reference Rust implementation:

```rust
fn anchored_gradient(util: f32) -> (u8, u8, u8) {
    let u = util.clamp(0.0, 1.0);
    let (start, end, t) = if u < 0.60 {
        ((46, 184, 46), (230, 184, 0), u / 0.60)
    } else if u < 0.85 {
        ((230, 184, 0), (204, 41, 41), (u - 0.60) / 0.25)
    } else {
        return (204, 41, 41);
    };
    let lerp = |a: u8, b: u8, t: f32| -> u8 {
        (a as f32 + t * (b as f32 - a as f32)).round() as u8
    };
    (lerp(start.0, end.0, t), lerp(start.1, end.1, t), lerp(start.2, end.2, t))
}
```

For `util > 1.0` the gradient is bypassed entirely — background is solid red `#cc2929` and the glyph is `!`.

## Rendering pipeline

One render per `WM_APP_POLL` (i.e., one per polling tick — every 60–300 s):

1. Compute the visual: `(bg_color, glyph)` from `last_status` and `last_sample`.
2. Create a fresh GDI+ `Bitmap` 16×16, pixel format `PixelFormat32bppARGB`.
3. Create a `Graphics` from the bitmap. Enable text anti-aliasing (`Graphics::set_text_rendering_hint(AntiAliasGridFit)`).
4. Fill the bitmap with `bg_color` via `Graphics::Clear`.
5. Build a `GraphicsPath` for the glyph using `AddString` with the chosen font.
6. Measure the path's bounds, compute centered position `(x, y) = ((16 - w) / 2, (16 - h) / 2)`, translate the path.
7. Draw the outline: `DrawPath` with a 1 px black `Pen`.
8. Fill the interior: `FillPath` with a white `SolidBrush`.
9. Convert to `HICON` via `Bitmap::GetHICON`.
10. Pass the new HICON to `Shell_NotifyIconW(NIM_MODIFY)`.
11. If step 10 succeeds, destroy the previous frame's HICON via `DestroyIcon`. If it fails, destroy the *new* HICON (which the shell isn't referencing) and keep the old one displayed.

GDI+ objects (`Bitmap`, `Graphics`, `GraphicsPath`, `Pen`, `SolidBrush`, `FontFamily`, `Font`) are stack-local per render. They drop automatically when the render function returns. The HICON is the *only* object that outlives the render — see "Resource management" below.

## Module layout changes from Stage 3

```text
src/
  main.rs                 — add GdiplusStartup at init, GdiplusShutdown at exit
  tray/
    icon.rs               — REPLACED: `IconSet` → `IconRenderer`
                            New module-private fns: anchored_gradient, compute_visual, draw_glyph
    window.rs             — `drain_and_redraw` now calls renderer.render() and
                            manages HICON lifecycle (destroy old after modify succeeds)
    poller.rs             — unchanged
    mod.rs                — wire up `IconRenderer` instead of `IconSet`
  api/                    — unchanged
  render.rs               — unchanged (the textual `LastStatus` formatting is independent)
  watch.rs                — unchanged
```

### `IconRenderer`

```rust
pub struct IconRenderer;

impl IconRenderer {
    pub fn new() -> Self { Self }

    /// Render the icon for the current state. Returns a fresh HICON owned by the caller.
    /// The caller MUST call DestroyIcon when done with it.
    pub fn render(
        &self,
        status: &LastStatus,
        sample: Option<&UsageSnapshot>,
    ) -> Result<HICON> {
        let (bg, glyph) = compute_visual(status, sample);
        // ... GDI+ calls ...
        Ok(hicon)
    }
}
```

The renderer holds no state of its own — every render is a pure function of `(status, sample)`. We keep it as a struct (rather than free functions) so that if Stage 5+ wants to cache an `HFONT` or `FontFamily` for performance, the field has a natural home.

`compute_visual` is the pure-function core (no Win32, no GDI):

```rust
fn compute_visual(status: &LastStatus, sample: Option<&UsageSnapshot>)
    -> ((u8, u8, u8), Glyph)
{
    match status {
        LastStatus::Initial | LastStatus::RateLimited | LastStatus::Error(_)
            => ((0x80, 0x80, 0x80), Glyph::Question),
        LastStatus::Ok => match sample.and_then(util_max) {
            Some(u) if u > 1.0 => ((0xCC, 0x29, 0x29), Glyph::Bang),
            Some(u) => (anchored_gradient(u), Glyph::Digits(percent_int(u))),
            None    => ((0x80, 0x80, 0x80), Glyph::Question),
        },
    }
}

enum Glyph { Digits(u8), Bang, Question }
```

`percent_int` rounds to nearest integer in 0..=100. `util_max` computes `max(five_hour.util, seven_day.util)`.

### `TrayState` changes

```rust
pub struct TrayState {
    pub last_sample: Option<(UsageSnapshot, DateTime<Utc>)>,
    pub last_status: LastStatus,
    pub renderer: IconRenderer,        // NEW: replaces icons: IconSet
    pub current_hicon: Option<HICON>,   // NEW: tracked for destruction
    pub rx: Receiver<PollEvent>,
    pub shutdown: Arc<AtomicBool>,
}
```

On every `WM_APP_POLL`, `drain_and_redraw` runs the steps from "Rendering pipeline" above and updates `current_hicon`.

### GDI+ initialization in `main.rs`

GDI+ requires a one-time process-wide init/shutdown pair. We wrap the token in a Drop guard so cleanup happens even on `?`-early-return paths:

```rust
struct GdiplusGuard(usize);

impl GdiplusGuard {
    fn init() -> anyhow::Result<Self> {
        let mut token: usize = 0;
        let input = Gdiplus::GdiplusStartupInput {
            GdiplusVersion: 1,
            ..Default::default()
        };
        let status = unsafe {
            Gdiplus::GdiplusStartup(&mut token, &input, std::ptr::null_mut())
        };
        if status != Gdiplus::Ok {
            anyhow::bail!("GdiplusStartup failed: {status:?}");
        }
        Ok(Self(token))
    }
}

impl Drop for GdiplusGuard {
    fn drop(&mut self) {
        unsafe { Gdiplus::GdiplusShutdown(self.0) };
    }
}

fn main() -> anyhow::Result<()> {
    // ... existing AttachConsole / CLI parsing / subscriber setup ...

    let _gdiplus = GdiplusGuard::init()?;

    // dispatch: tray / once / watch ...

    Ok(())
} // GdiplusGuard drops here → GdiplusShutdown runs, even on early returns.
```

Notes for a Rust beginner:
- `GdiplusStartup` is global per-process — one call total, shared across all threads.
- The `gdiplus_token` is opaque — hold onto it, pass back to `GdiplusShutdown`.
- `GdiplusShutdown` invalidates every GDI+ object created since startup. The Drop guard guarantees it runs after `main`'s body completes (whether by `Ok(())`, by `?`, or by panic unwinding).
- Naming the guard `_gdiplus` (leading underscore) keeps the value alive without a clippy "unused variable" warning. A bare `_` would Drop *immediately* — guard pattern requires a name.
- `--once` and `--watch` modes never touch GDI+ — initializing it for them is a small waste (~1 ms startup, no leak thanks to the guard). Simpler than conditionalizing the init by mode.

## Resource management

### HICON lifecycle

This is the most important correctness concern in Stage 4. Stage 3 created 4 HICONs at startup and freed them all on Drop. Stage 4 creates a *fresh* HICON every 60–300 seconds, so a leak would accumulate ~17 handles/hour ≈ 400/day.

Rules:
- `TrayState::current_hicon: Option<HICON>` is the single owner of the currently-displayed icon.
- Each `drain_and_redraw` invocation:
  1. Renders a *new* HICON (call it `next`).
  2. Calls `NIM_MODIFY` with `next`.
  3. If the shell accepted: destroys the *old* HICON (`current_hicon.take()`), stores `next`.
  4. If the shell rejected: destroys `next`, leaves `current_hicon` untouched.
- On shutdown the final `current_hicon` is destroyed via `impl Drop for TrayState`:

  ```rust
  impl Drop for TrayState {
      fn drop(&mut self) {
          if let Some(h) = self.current_hicon.take() {
              unsafe { let _ = DestroyIcon(h); }
          }
      }
  }
  ```

  This mirrors Stage 3's `impl Drop for IconSet`. The `Box::from_raw(state_ptr)` call in `WM_NCDESTROY` (carried over from Stage 3) triggers this Drop automatically — no change to the `WM_NCDESTROY` handler.

This ordering matters: we destroy the *old* HICON only after the shell has acknowledged the new one. Destroying first risks the shell briefly trying to render a freed HICON.

### GDI+ object lifecycle

GDI+ `Bitmap`, `Graphics`, etc. in the `windows` crate are RAII-friendly: when the binding wraps the raw GDI+ object, Drop emits the corresponding `Delete` call. No explicit cleanup needed inside `render()`.

Important: the HICON returned by `Bitmap::GetHICON` is an **independent copy** of the pixel data. Destroying the Bitmap does *not* destroy the HICON. They're separate handles.

## Error handling

- `GdiplusStartup` failure in `main()`: fatal, app exits non-zero.
- Per-render failure (Bitmap creation, GetHICON, etc.): log `tracing::warn!`, leave `current_hicon` unchanged. Next poll retries — at worst the user sees a stale icon for one polling interval.
- `DestroyIcon` failure: log warning, continue. (Bounded leak — at most one HICON per failed destroy.)
- `Shell_NotifyIconW` failure handling carries over from Stage 3 unchanged (Explorer-restart case = log warning, don't recover).

## New runtime dependencies

No new crates. We expand the existing `windows` crate's feature list:

```toml
windows = { version = "0.58", features = [
    "Win32_Foundation",
    "Win32_UI_Shell",
    "Win32_UI_WindowsAndMessaging",
    "Win32_System_LibraryLoader",
    "Win32_Graphics_Gdi",
    "Win32_Graphics_GdiPlus",     # NEW for Stage 4
]}
```

`gdiplus.dll` ships with every supported Windows version. The added bindings cost ~50–100 KB of binary size.

## Testing

- **No unit tests for GDI+ rendering itself.** Win32 graphics doesn't unit-test cleanly. Validation is manual smoke test.
- **One new unit test** for `anchored_gradient(util) → (r,g,b)` — verifies the three anchor points and a few interpolation midpoints.
- **One new unit test** for `compute_visual(status, sample) → (color, glyph)` — verifies state-machine branching (Initial/RateLimited/Error → gray+?; Ok with util > 1 → red+!; Ok with util ≤ 1 → gradient+digits; Ok with no sample → gray+?).
- The existing 13 Stage 1–3 tests must continue to pass.

## Stage 4 deliverable / verification

End-to-end checks before tagging `v0.4.0`:

- `cargo fmt --check` clean.
- `cargo clippy --all-targets -- -D warnings` clean.
- `cargo test` — all tests pass (existing 13 + ~2 new = ~15).
- `cargo build --release` → `target\release\claude-usage-tray.exe`, expected ~4.5 MB.
- Double-click the .exe → tray icon appears showing gray + `?`, transitions to a colored 2-digit icon within one polling interval.
- Hover the icon → tooltip still shows `5h: NN%   7d: NN%` two-line layout from Stage 3.
- Tooltip and icon agree: tooltip `5h: 57%   7d: 42%` pairs with an icon showing `57` on green-leaning-yellow background.
- Force a rate-limit (poll twice within 60 s): icon goes back to gray + `?`.
- Force `util > 1.0` (inject a synthetic sample if needed during development): icon shows red + `!`.
- Right-click + Quit: icon disappears within ~1 second; process exits cleanly.
- Run for ≥1 hour, then check `Process Explorer` → handle count stable, no GDI handle leak.
- `claude-usage-tray.exe --once` and `--watch` still produce Stage 2 output on the terminal.
- Tag `v0.4.0` and push.

## Carry-overs from Stage 3 (unchanged)

- Polling cadence (60/120/300 s; default 120 s).
- Threading model (UI thread + polling thread + mpsc + `PostMessageW`).
- Tooltip format and shutdown sequence.
- Color thresholds (60/85).
- State machine (`Initial` / `Ok` / `RateLimited` / `Error`).
- Stage 3's `IconSet` is **replaced** by `IconRenderer` — they do not coexist.

## Stage 4 enabling Stage 5

Stage 5 (calibration math) will compute util against a locally-calibrated cap from `~/.claude/projects/` JSONL data, rather than the API's reported `utilization`. The rendering pipeline is unaffected — `compute_visual` continues to take a `util ∈ [0.0, ∞)`; only its *source* changes. No icon code touches Stage 5.

## Open questions deferred to implementation

- **Exact font size and weight.** Segoe UI Black 10 pt is the starting point. Segoe UI Bold 11 pt or a slightly smaller bitmap-friendly weight may render cleaner at 16×16. Decide after seeing actual output.
- **Anti-aliasing setting.** GDI+ offers `AntiAliasGridFit`, `AntiAlias`, and `SingleBitPerPixel`. At 16×16 the gridfit variant is usually best, but if it produces muddy edges fall back to `SingleBitPerPixel` (no AA, crisp pixels).
- **Font fallback.** If Segoe UI is not present (uncommon but possible on stripped-down Windows builds), default to `FontFamily::GenericSansSerif()`.
- **Optional GDI+ resource caching.** Holding an `HFONT` or `FontFamily` on the `IconRenderer` to avoid recreation per frame is a possible perf optimization. Negligible at 60-second poll cadence; revisit if the dashboard window (Stage 6) drives faster redraws.
