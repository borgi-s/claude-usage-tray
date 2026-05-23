# Stage 4 — GDI+ Rendered Percentage Icon Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace Stage 3's four pre-built solid-color `IconSet` HICONs with a GDI+-rendered tray icon that shows the current utilization as a 1–2 character glyph on a color-graded background. Tooltip, polling, menu, and shutdown stay byte-identical to Stage 3.

**Architecture:** Add the `Win32_Graphics_GdiPlus` feature to the `windows` crate. Initialize GDI+ in `main()` via a Drop-guard so cleanup runs on any exit path. Replace `tray::icon::IconSet` with `tray::icon::IconRenderer`, a stateless struct that produces a fresh `HICON` per redraw using GDI+ `Bitmap` + `Graphics` + `GraphicsPath`. Track the currently-displayed HICON on `TrayState` so we can destroy the previous one after `NIM_MODIFY` succeeds.

**Tech Stack:** Rust stable, `windows` crate v0.58 (already in tree), GDI+ via the `Win32_Graphics_GdiPlus` feature. No new third-party dependencies.

**Reference spec:** `docs/superpowers/specs/2026-05-23-stage-4-gdi-icon-design.md`

---

## File Structure

### Created
- (none — Stage 4 modifies only existing files)

### Modified
- `Cargo.toml` — add `Win32_Graphics_GdiPlus` to the `windows` crate's feature list
- `src/main.rs` — add `GdiplusGuard` struct + `init()` call at the top of `main`
- `src/tray/icon.rs` — replace `IconSet` with `IconRenderer` and helper pure functions; inline tests
- `src/tray/window.rs` — change `TrayState` fields, rewrite `drain_and_redraw` for HICON lifecycle, add `impl Drop for TrayState`
- `src/tray/mod.rs` — construct `IconRenderer` instead of `IconSet`; render initial icon before `icon::add`

---

## Beginner notes (read first if you're new to Rust + Win32)

A few patterns you'll see repeatedly in this plan:

- **`unsafe` blocks:** GDI+ functions in the `windows` crate are FFI calls into C — Rust can't prove the calls are sound, so every call is wrapped in `unsafe { ... }`. The block is small and localized; we're saying "trust me, the Win32 contract is upheld here."
- **Raw pointers `*mut T`:** GDI+ uses out-parameters (you pass a `&mut *mut GpFoo` and the function fills it). Always initialize the pointer to `std::ptr::null_mut()` first, then pass `&mut it`.
- **`Status` return codes:** every GDI+ function returns a `Status` enum. `Status::Ok` (value 0) is success; anything else is failure. We bail on non-Ok with `anyhow::bail!`.
- **HICON is a Win32 handle**, not a Rust struct. It does NOT free itself on drop. You must call `DestroyIcon(h)` explicitly when done. The whole "HICON lifecycle" dance in Task 6 exists for this reason.
- **GDI+ object lifetimes:** `Bitmap`, `Graphics`, `Pen`, etc. need explicit `GdipDeleteXxx` calls. In this plan we follow the pattern: create → use → delete, all in one function, so a function that fails to clean up is a localized bug.

If you've never written Win32 FFI in Rust before, `cargo doc --open` after Task 1 (with the GdiPlus feature enabled) is highly useful — it shows the actual signatures and types in the `windows` crate.

---

## Task 1: Add `Win32_Graphics_GdiPlus` feature to Cargo.toml

**Files:**
- Modify: `Cargo.toml:23-30`

- [ ] **Step 1: Read the current Cargo.toml**

The current `[dependencies]` section lists the `windows` crate with six features. We add a seventh.

- [ ] **Step 2: Edit `Cargo.toml`**

Replace this block:

```toml
windows = { version = "0.58", features = [
    "Win32_Foundation",
    "Win32_Graphics_Gdi",
    "Win32_System_Console",
    "Win32_System_LibraryLoader",
    "Win32_UI_Shell",
    "Win32_UI_WindowsAndMessaging",
] }
```

with:

```toml
windows = { version = "0.58", features = [
    "Win32_Foundation",
    "Win32_Graphics_Gdi",
    "Win32_Graphics_GdiPlus",
    "Win32_System_Console",
    "Win32_System_LibraryLoader",
    "Win32_UI_Shell",
    "Win32_UI_WindowsAndMessaging",
] }
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo build`
Expected: builds cleanly. First build with the new feature will download/compile additional bindings — that's normal and only happens once.

- [ ] **Step 4: Commit**

```
git add Cargo.toml Cargo.lock
git commit -m "chore(stage-4): enable windows crate's Win32_Graphics_GdiPlus feature"
```

---

## Task 2: Implement `anchored_gradient` pure function with unit tests

The first piece of real Stage 4 code. Pure function, fully testable, no Win32.

**Files:**
- Modify: `src/tray/icon.rs` — add `anchored_gradient` + tests at the bottom of the file (before `solid_icon`, which we'll remove later)

- [ ] **Step 1: Add a failing test at the bottom of `src/tray/icon.rs`**

Append:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchored_gradient_anchors_match() {
        assert_eq!(anchored_gradient(0.00), (46, 184, 46));   // green
        assert_eq!(anchored_gradient(0.60), (230, 184, 0));   // yellow
        assert_eq!(anchored_gradient(0.85), (204, 41, 41));   // red
    }

    #[test]
    fn anchored_gradient_clamps_above_85_to_red() {
        assert_eq!(anchored_gradient(0.90), (204, 41, 41));
        assert_eq!(anchored_gradient(1.00), (204, 41, 41));
    }

    #[test]
    fn anchored_gradient_clamps_below_zero_to_green() {
        assert_eq!(anchored_gradient(-0.50), (46, 184, 46));
    }

    #[test]
    fn anchored_gradient_midpoint_of_green_to_yellow() {
        // 0.30 = halfway between 0.00 (green) and 0.60 (yellow).
        // R: 46 + 0.5*(230-46) = 138
        // G: 184 + 0.5*(184-184) = 184
        // B: 46 + 0.5*(0-46) = 23
        assert_eq!(anchored_gradient(0.30), (138, 184, 23));
    }

    #[test]
    fn anchored_gradient_midpoint_of_yellow_to_red() {
        // 0.725 = halfway between 0.60 (yellow) and 0.85 (red).
        // R: 230 + 0.5*(204-230) = 217
        // G: 184 + 0.5*(41-184) = 113 (rounded from 112.5)
        // B: 0 + 0.5*(41-0) = 21 (rounded from 20.5)
        assert_eq!(anchored_gradient(0.725), (217, 113, 21));
    }
}
```

- [ ] **Step 2: Verify the tests fail**

Run: `cargo test --lib anchored_gradient`
Expected: 5 compilation errors of the form "cannot find function `anchored_gradient` in this scope".

- [ ] **Step 3: Add the implementation**

In `src/tray/icon.rs`, above the `#[cfg(test)]` block, add:

```rust
/// Map a util value in [0.0, ∞) to an RGB color using the anchored gradient:
///   0.00 → green (#2eb82e), 0.60 → yellow (#e6b800), 0.85+ → red (#cc2929).
/// Values below 0 clamp to green; values at/above 0.85 clamp to red.
/// Linear RGB interpolation between anchors.
pub(crate) fn anchored_gradient(util: f64) -> (u8, u8, u8) {
    let u = util.clamp(0.0, 1.0);
    let (start, end, t) = if u < 0.60 {
        ((46u8, 184u8, 46u8), (230u8, 184u8, 0u8), u / 0.60)
    } else if u < 0.85 {
        ((230u8, 184u8, 0u8), (204u8, 41u8, 41u8), (u - 0.60) / 0.25)
    } else {
        return (204, 41, 41);
    };
    let lerp = |a: u8, b: u8, t: f64| -> u8 {
        (a as f64 + t * (b as f64 - a as f64)).round() as u8
    };
    (
        lerp(start.0, end.0, t),
        lerp(start.1, end.1, t),
        lerp(start.2, end.2, t),
    )
}
```

Beginner note: `(46u8, 184u8, 46u8)` is a tuple of three `u8` values. The `u8` suffix on each literal is required so the compiler knows the type (otherwise it defaults to `i32` and the tuple types in the two branches don't match).

- [ ] **Step 4: Verify the tests pass**

Run: `cargo test --lib anchored_gradient`
Expected: 5 passed, 0 failed.

- [ ] **Step 5: Commit**

```
git add src/tray/icon.rs
git commit -m "feat(stage-4): add anchored_gradient color interpolation with tests"
```

---

## Task 3: Implement `compute_visual` pure function with unit tests

The state machine that decides what to render for any (status, sample) combination.

**Files:**
- Modify: `src/tray/icon.rs` — add `Glyph` enum, `util_max` helper, `compute_visual` function, and tests

- [ ] **Step 1: Add the failing tests**

In the `#[cfg(test)]` block from Task 2, add:

```rust
use crate::api::usage::{UsageBucket, UsageSnapshot};
use crate::render::LastStatus;
use chrono::Utc;

fn snap_with(five: Option<f64>, seven: Option<f64>) -> UsageSnapshot {
    UsageSnapshot {
        five_hour: five.map(|u| UsageBucket { utilization: u, resets_at: None }),
        seven_day: seven.map(|u| UsageBucket { utilization: u, resets_at: None }),
    }
}

#[test]
fn compute_visual_initial_is_gray_question() {
    let (bg, g) = compute_visual(&LastStatus::Initial, None);
    assert_eq!(bg, (0x80, 0x80, 0x80));
    assert!(matches!(g, Glyph::Question));
}

#[test]
fn compute_visual_rate_limited_is_gray_question_even_with_cached_sample() {
    let snap = snap_with(Some(0.50), Some(0.20));
    let (bg, g) = compute_visual(&LastStatus::RateLimited, Some(&snap));
    assert_eq!(bg, (0x80, 0x80, 0x80));
    assert!(matches!(g, Glyph::Question));
}

#[test]
fn compute_visual_error_is_gray_question() {
    let snap = snap_with(Some(0.50), None);
    let (bg, g) = compute_visual(&LastStatus::Error("network".into()), Some(&snap));
    assert_eq!(bg, (0x80, 0x80, 0x80));
    assert!(matches!(g, Glyph::Question));
}

#[test]
fn compute_visual_ok_with_no_sample_is_gray_question() {
    let (bg, g) = compute_visual(&LastStatus::Ok, None);
    assert_eq!(bg, (0x80, 0x80, 0x80));
    assert!(matches!(g, Glyph::Question));
}

#[test]
fn compute_visual_ok_under_100_uses_gradient_and_digits() {
    let snap = snap_with(Some(0.57), Some(0.42));
    let (bg, g) = compute_visual(&LastStatus::Ok, Some(&snap));
    // max = 0.57, in green→yellow range
    assert_eq!(bg, anchored_gradient(0.57));
    assert!(matches!(g, Glyph::Digits(57)));
}

#[test]
fn compute_visual_ok_max_picks_larger_bucket() {
    // 5h is smaller, 7d should win
    let snap = snap_with(Some(0.20), Some(0.80));
    let (_, g) = compute_visual(&LastStatus::Ok, Some(&snap));
    assert!(matches!(g, Glyph::Digits(80)));
}

#[test]
fn compute_visual_ok_over_100_is_red_bang() {
    let snap = snap_with(Some(1.10), None);
    let (bg, g) = compute_visual(&LastStatus::Ok, Some(&snap));
    assert_eq!(bg, (0xCC, 0x29, 0x29));
    assert!(matches!(g, Glyph::Bang));
}

#[test]
fn compute_visual_ok_one_bucket_missing_uses_the_other() {
    let snap = snap_with(Some(0.65), None);
    let (_, g) = compute_visual(&LastStatus::Ok, Some(&snap));
    assert!(matches!(g, Glyph::Digits(65)));
}
```

- [ ] **Step 2: Verify tests fail**

Run: `cargo test --lib compute_visual`
Expected: compilation errors — `Glyph`, `compute_visual` not in scope.

- [ ] **Step 3: Add the implementation**

In `src/tray/icon.rs`, add at the top (just below the existing `use` statements):

```rust
use crate::render::LastStatus;
```

Then add (above the `#[cfg(test)]` block but below `anchored_gradient`):

```rust
/// What the icon's glyph slot should show. The `Digits` variant carries a 0..=100
/// percentage; `Bang` is the `!` for over-100% util; `Question` is the `?` for
/// no-data states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Glyph {
    Digits(u8),
    Bang,
    Question,
}

/// Pure: pick the max utilization across the two buckets.
/// Returns None only if neither bucket has data.
fn util_max(snap: &UsageSnapshot) -> Option<f64> {
    let h5 = snap.five_hour.as_ref().map(|b| b.utilization);
    let d7 = snap.seven_day.as_ref().map(|b| b.utilization);
    match (h5, d7) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

/// Pure: decide background color and glyph from the current poll state.
///
/// Maps:
/// - Initial / RateLimited / Error / (Ok + no sample) → gray + `?`
/// - Ok + util > 1.0                                   → red + `!`
/// - Ok + util ≤ 1.0                                   → gradient + digits
pub(crate) fn compute_visual(
    status: &LastStatus,
    sample: Option<&UsageSnapshot>,
) -> ((u8, u8, u8), Glyph) {
    match status {
        LastStatus::Initial | LastStatus::RateLimited | LastStatus::Error(_) => {
            ((0x80, 0x80, 0x80), Glyph::Question)
        }
        LastStatus::Ok => match sample.and_then(util_max) {
            Some(u) if u > 1.0 => ((0xCC, 0x29, 0x29), Glyph::Bang),
            Some(u) => (anchored_gradient(u), Glyph::Digits(percent_int(u))),
            None => ((0x80, 0x80, 0x80), Glyph::Question),
        },
    }
}

/// Round a util in [0.0, 1.0] to an integer percent in 0..=100.
fn percent_int(util: f64) -> u8 {
    let pct = (util.clamp(0.0, 1.0) * 100.0).round();
    pct as u8
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test --lib`
Expected: all tests pass (5 from Task 2 + 8 new = 13 new from Stage 4, plus the existing 13 from Stages 1–3 = 26 total).

- [ ] **Step 5: Commit**

```
git add src/tray/icon.rs
git commit -m "feat(stage-4): add compute_visual state machine with tests"
```

---

## Task 4: Add `GdiplusGuard` to `main.rs`

Compile-only step — no behavioral change yet because nothing uses GDI+ yet. We add the lifecycle scaffolding so Task 5 has somewhere to plug in.

**Files:**
- Modify: `src/main.rs` — add `GdiplusGuard` struct and call from `main`

- [ ] **Step 1: Add the import**

In `src/main.rs`, alongside the existing `windows::Win32::System::Console::...` imports, add (anywhere above `fn main`):

```rust
use windows::Win32::Graphics::GdiPlus::{
    GdiplusShutdown, GdiplusStartup, GdiplusStartupInput, Status,
};
```

- [ ] **Step 2: Add the guard struct above `fn main`**

```rust
/// RAII guard that initializes GDI+ in `init()` and shuts it down on drop.
/// We hold one for the whole process lifetime so cleanup runs on every exit path
/// (including `?` early-returns and panic unwinding).
struct GdiplusGuard(usize);

impl GdiplusGuard {
    fn init() -> Result<Self> {
        let mut token: usize = 0;
        let input = GdiplusStartupInput {
            GdiplusVersion: 1,
            ..Default::default()
        };
        // SAFETY: token is on the stack and the input pointer is valid.
        // GdiplusStartup writes the token and returns a Status code.
        let status = unsafe { GdiplusStartup(&mut token, &input, std::ptr::null_mut()) };
        if status != Status(0) {
            anyhow::bail!("GdiplusStartup failed with status {:?}", status);
        }
        Ok(Self(token))
    }
}

impl Drop for GdiplusGuard {
    fn drop(&mut self) {
        // SAFETY: token was obtained from a successful GdiplusStartup and we are
        // the sole owner. After shutdown, no more GDI+ calls happen.
        unsafe { GdiplusShutdown(self.0) };
    }
}
```

Beginner note: `Status(0)` is GDI+'s success code (the `Status` newtype wraps an i32). If the `windows` crate's bindings happen to expose this as a named constant like `Status::Ok`, prefer that — check with rust-analyzer / `cargo doc`.

- [ ] **Step 3: Initialize the guard inside `main`**

Find this line near the top of `fn main()`:

```rust
let cli = Cli::parse();
```

Immediately after it, add:

```rust
let _gdiplus = GdiplusGuard::init()?;
```

The leading underscore in `_gdiplus` tells Rust "I'm intentionally not using this name — but keep the value alive until the end of scope." A bare `_` would Drop the guard *immediately*, which we don't want.

- [ ] **Step 4: Verify it compiles**

Run: `cargo build`
Expected: clean build. Possible name mismatch on `Status(0)` / `GdiplusStartupInput`'s field names — if so, look up the exact identifier in the `windows` crate (right-click → "Go to Definition" in your IDE) and substitute.

- [ ] **Step 5: Smoke test the existing modes**

Run: `cargo run -- --once`
Expected: prints `5h: NN%` / `7d: NN%` (or `(no data)` / rate-limited error from your token state) — same Stage 2 behavior. GDI+ init/shutdown happens silently around it.

- [ ] **Step 6: Commit**

```
git add src/main.rs
git commit -m "chore(stage-4): add GdiplusGuard to main (init/shutdown lifecycle)"
```

---

## Task 5: Stub `IconRenderer` that draws a flat-colored 16×16 HICON

This is the first GDI+ rendering code. The stub only does the bitmap → graphics → clear → HICON pipeline (no text yet). When this compiles and produces an HICON, we know the GDI+ binding shapes are right.

**Files:**
- Modify: `src/tray/icon.rs` — add `IconRenderer` struct and `render` method (stub: solid background, no glyph)

- [ ] **Step 1: Add imports for GDI+ at the top of `src/tray/icon.rs`**

Below the existing imports, add:

```rust
use windows::Win32::Graphics::GdiPlus::{
    GdipCreateBitmapFromScan0, GdipCreateHICONFromBitmap, GdipDisposeImage, GdipGetImageGraphicsContext,
    GdipGraphicsClear, GpBitmap, GpGraphics, GpImage, PixelFormat32bppARGB, Status,
};
```

Note: if any name doesn't resolve, search the `windows::Win32::Graphics::GdiPlus` module via `cargo doc --open --no-deps` and pick the closest match. The shapes are consistent: `GdipCreateXxx`, `GdipDeleteXxx`, `GpFoo` for opaque pointer types.

- [ ] **Step 2: Add the `IconRenderer` struct + `render` (stub) method**

Add the following ABOVE the `#[cfg(test)]` mod block:

```rust
/// Stateless renderer that produces a fresh 16×16 HICON per call.
/// Each render does a complete GDI+ pipeline: create bitmap → fill → convert → return HICON.
/// Caller is responsible for `DestroyIcon` on the returned handle.
pub struct IconRenderer;

impl IconRenderer {
    pub fn new() -> Self {
        Self
    }

    /// Render the current visual state to a fresh HICON.
    /// Returns an error if any GDI+ call fails.
    pub fn render(
        &self,
        status: &LastStatus,
        sample: Option<&UsageSnapshot>,
    ) -> Result<HICON> {
        let ((r, g, b), _glyph) = compute_visual(status, sample);

        // 1) Create a 16x16 ARGB bitmap.
        let mut bitmap: *mut GpBitmap = std::ptr::null_mut();
        // SAFETY: out-pointer is valid; stride 0 means "let GDI+ pick"; scan0 null means
        // "allocate a fresh buffer." PixelFormat32bppARGB requests premultiplied ARGB.
        let s = unsafe {
            GdipCreateBitmapFromScan0(16, 16, 0, PixelFormat32bppARGB, std::ptr::null_mut(), &mut bitmap)
        };
        if s != Status(0) {
            anyhow::bail!("GdipCreateBitmapFromScan0 failed: {s:?}");
        }

        // 2) Get a Graphics for that bitmap.
        let mut graphics: *mut GpGraphics = std::ptr::null_mut();
        // SAFETY: bitmap was just created successfully; cast to GpImage is the GDI+ idiom.
        let s = unsafe { GdipGetImageGraphicsContext(bitmap as *mut GpImage, &mut graphics) };
        if s != Status(0) {
            // SAFETY: bitmap is valid; we own it.
            unsafe { GdipDisposeImage(bitmap as *mut GpImage); }
            anyhow::bail!("GdipGetImageGraphicsContext failed: {s:?}");
        }

        // 3) Fill background. GDI+ Argb format is 0xAARRGGBB.
        let argb = 0xFF00_0000u32 | (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b);
        // SAFETY: graphics is valid; argb is u32.
        let s = unsafe { GdipGraphicsClear(graphics, argb as i32) };
        if s != Status(0) {
            // SAFETY: both handles valid.
            unsafe {
                windows::Win32::Graphics::GdiPlus::GdipDeleteGraphics(graphics);
                GdipDisposeImage(bitmap as *mut GpImage);
            }
            anyhow::bail!("GdipGraphicsClear failed: {s:?}");
        }

        // TODO Task 7: draw the glyph here.

        // 4) Convert to HICON. The HICON owns its pixel data independently
        //    from the bitmap, so we can dispose the bitmap right after.
        let mut hicon = HICON::default();
        // SAFETY: bitmap valid; hicon out-pointer valid.
        let s = unsafe { GdipCreateHICONFromBitmap(bitmap, &mut hicon) };

        // 5) Clean up GDI+ objects regardless of conversion outcome.
        // SAFETY: both handles valid.
        unsafe {
            windows::Win32::Graphics::GdiPlus::GdipDeleteGraphics(graphics);
            GdipDisposeImage(bitmap as *mut GpImage);
        }

        if s != Status(0) {
            anyhow::bail!("GdipCreateHICONFromBitmap failed: {s:?}");
        }
        Ok(hicon)
    }
}

impl Default for IconRenderer {
    fn default() -> Self {
        Self::new()
    }
}
```

Beginner notes:
- `*mut GpBitmap` is a raw pointer — the `windows` crate uses these to wrap opaque GDI+ handles.
- The pattern `let mut x: *mut Foo = std::ptr::null_mut(); unsafe { CreateThing(&mut x) };` is standard for Win32 out-parameters.
- `argb as i32` (the `i32` cast) is because `GdipGraphicsClear` takes an i32 ARGB color. The bit pattern is identical between u32 and i32 — it's just a type coercion.

- [ ] **Step 3: Verify it compiles**

Run: `cargo check`
Expected: clean. If a function/type name doesn't resolve, find it via rust-analyzer or `cargo doc --open`. Common adjustments:
- `Status` might be a unit struct vs newtype — try `Status(0)` first, fall back to `Status::Ok`.
- `PixelFormat32bppARGB` may be a constant or an i32; if it's an i32 you can pass it directly.

- [ ] **Step 4: Verify tests still pass**

Run: `cargo test --lib`
Expected: all tests pass. (The new code isn't wired into the tray yet; existing behavior is unaffected.)

- [ ] **Step 5: Commit**

```
git add src/tray/icon.rs
git commit -m "feat(stage-4): IconRenderer stub — flat color HICON via GDI+ pipeline"
```

---

## Task 6: Wire `IconRenderer` into `TrayState` + add HICON lifecycle

Replace the `IconSet` field with `IconRenderer + current_hicon`. Update `drain_and_redraw` to render a fresh HICON, swap it in, and destroy the old one only after `NIM_MODIFY` succeeds. Add `impl Drop for TrayState` to clean up the final HICON on shutdown.

This task touches three files and is the riskiest change. Test by running the .exe at the end — you should see flat-color icons (no digits yet) that update color on each poll.

**Files:**
- Modify: `src/tray/window.rs` — `TrayState` fields, `drain_and_redraw`, `Drop`
- Modify: `src/tray/mod.rs` — replace `icon::IconSet::new` with `icon::IconRenderer::new`, replace `peek_initial_icon` to render via the new renderer
- Modify: `src/tray/icon.rs` — no code change, but the `IconSet` struct stays in place for now (we delete it in Task 8 after the new path is working)

- [ ] **Step 1: Update `TrayState` fields in `src/tray/window.rs`**

Find this struct (around line 36):

```rust
pub struct TrayState {
    pub last_sample: Option<(UsageSnapshot, DateTime<Utc>)>,
    pub last_status: LastStatus,
    pub icons: IconSet,
    pub rx: Receiver<PollEvent>,
    pub shutdown: Arc<AtomicBool>,
}
```

Replace with:

```rust
pub struct TrayState {
    pub last_sample: Option<(UsageSnapshot, DateTime<Utc>)>,
    pub last_status: LastStatus,
    pub renderer: IconRenderer,
    pub current_hicon: Option<HICON>,
    pub rx: Receiver<PollEvent>,
    pub shutdown: Arc<AtomicBool>,
}
```

And update the import line at the top:

```rust
use crate::tray::icon::{self, IconSet};
```

→

```rust
use crate::tray::icon::{self, IconRenderer};
```

- [ ] **Step 2: Add `impl Drop for TrayState`**

Add directly below the `pub struct TrayState { ... }` definition:

```rust
impl Drop for TrayState {
    fn drop(&mut self) {
        if let Some(h) = self.current_hicon.take() {
            // SAFETY: we own this handle (set by drain_and_redraw / the initial render).
            unsafe {
                let _ = windows::Win32::UI::WindowsAndMessaging::DestroyIcon(h);
            }
        }
    }
}
```

This runs automatically when the `Box::from_raw(state_ptr)` in `WM_NCDESTROY` drops the Box. No change to `WM_NCDESTROY` itself.

- [ ] **Step 3: Rewrite `drain_and_redraw` to do the HICON swap**

Find `fn drain_and_redraw` (around line 186). Replace its body:

```rust
fn drain_and_redraw(hwnd: HWND, state: &mut TrayState) {
    // Drain all queued events, keeping the most recent.
    while let Ok(event) = state.rx.try_recv() {
        match event {
            PollEvent::Ok(snap) => {
                state.last_sample = Some((snap, Utc::now()));
                state.last_status = LastStatus::Ok;
            }
            PollEvent::RateLimited => {
                state.last_status = LastStatus::RateLimited;
            }
            PollEvent::Error(msg) => {
                state.last_status = LastStatus::Error(msg);
            }
        }
    }

    let sample = state.last_sample.as_ref().map(|(s, _)| s);

    // Render a fresh HICON for the current state.
    let next_hicon = match state.renderer.render(&state.last_status, sample) {
        Ok(h) => h,
        Err(e) => {
            tracing::warn!(error = %e, "IconRenderer::render failed, keeping previous icon");
            // Still refresh the tooltip — text-only update.
            let tooltip = format_tooltip(&state.last_status, state.last_sample.as_ref(), Utc::now());
            if let Some(current) = state.current_hicon {
                icon::modify(hwnd, WM_APP_TRAYICON, current, &tooltip);
            }
            return;
        }
    };

    let tooltip = format_tooltip(&state.last_status, state.last_sample.as_ref(), Utc::now());
    icon::modify(hwnd, WM_APP_TRAYICON, next_hicon, &tooltip);

    // Swap in the new HICON. Destroy the previous one (if any) only AFTER NIM_MODIFY
    // has been called with the new one — otherwise the shell might briefly point at a freed handle.
    if let Some(prev) = state.current_hicon.replace(next_hicon) {
        // SAFETY: previous handle we owned; no longer referenced by the shell after the
        // NIM_MODIFY call above.
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::DestroyIcon(prev);
        }
    }
}
```

Beginner note: `state.current_hicon.replace(next_hicon)` stores `next_hicon` in the Option and returns the previous value (an `Option<HICON>`). The `if let Some(prev) = ...` then handles the case where there was an old icon to destroy.

- [ ] **Step 4: Update `src/tray/mod.rs` to construct `IconRenderer`**

Find `pub fn run` (around line 20). Replace this block:

```rust
let creds = load_from_default_path()?;
let hinst = window::current_hinstance()?;
let icons = icon::IconSet::new(hinst)?;

let shutdown = Arc::new(AtomicBool::new(false));
let (tx, rx) = mpsc::channel();

let state = Box::new(window::TrayState {
    last_sample: None,
    last_status: LastStatus::Initial,
    icons,
    rx,
    shutdown: shutdown.clone(),
});
```

with:

```rust
let creds = load_from_default_path()?;
let hinst = window::current_hinstance()?;
let renderer = icon::IconRenderer::new();

let shutdown = Arc::new(AtomicBool::new(false));
let (tx, rx) = mpsc::channel();

let state = Box::new(window::TrayState {
    last_sample: None,
    last_status: LastStatus::Initial,
    renderer,
    current_hicon: None,
    rx,
    shutdown: shutdown.clone(),
});
```

The `hinst` variable is no longer used by the renderer — but `window::current_hinstance` is still called for `RegisterClassExW`. Leave `hinst` as-is for now; `cargo clippy` would warn if it became fully unused.

- [ ] **Step 5: Replace `peek_initial_icon` with a fresh render**

The current `peek_initial_icon` in `src/tray/mod.rs` (around line 67) was reading the pre-built `IconSet`. We need to render a fresh initial icon and store it in `current_hicon`.

Replace the call site near the bottom of `pub fn run`:

```rust
// Build initial tooltip and register the tray icon.
let initial_tooltip = window::format_tooltip(&LastStatus::Initial, None, chrono::Utc::now());
{
    // Borrow the icons through GWLP_USERDATA-owned state for the initial add.
    let initial_icon = peek_initial_icon(hwnd);
    icon::add(
        hwnd,
        window::WM_APP_TRAYICON,
        initial_icon,
        &initial_tooltip,
    )?;
}
```

with:

```rust
// Build initial tooltip and register the tray icon with a freshly-rendered HICON.
let initial_tooltip = window::format_tooltip(&LastStatus::Initial, None, chrono::Utc::now());
render_and_store_initial_icon(hwnd, &initial_tooltip)?;
```

Then replace the `fn peek_initial_icon` definition at the bottom of the file:

```rust
/// Peek at the window's TrayState long enough to retrieve its initial icon.
/// Used only at startup, immediately after `window::create`.
fn peek_initial_icon(
    hwnd: windows::Win32::Foundation::HWND,
) -> windows::Win32::UI::WindowsAndMessaging::HICON {
    use windows::Win32::UI::WindowsAndMessaging::{GetWindowLongPtrW, GWLP_USERDATA};
    let state_ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const window::TrayState;
    // SAFETY: pointer set by `create`; window is on this thread; we read only.
    let state = unsafe { &*state_ptr };
    state.icons.for_state(&state.last_status, None)
}
```

with:

```rust
/// Render the initial gray-question icon and register it with the shell.
/// Stores the HICON in `state.current_hicon` so the next render can destroy it.
fn render_and_store_initial_icon(
    hwnd: windows::Win32::Foundation::HWND,
    tooltip: &[u16],
) -> anyhow::Result<()> {
    use windows::Win32::UI::WindowsAndMessaging::{GetWindowLongPtrW, GWLP_USERDATA};
    let state_ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut window::TrayState;
    // SAFETY: pointer set by `create`; window is on this thread; sole owner.
    let state = unsafe { &mut *state_ptr };

    let hicon = state.renderer.render(&state.last_status, None)?;
    state.current_hicon = Some(hicon);
    icon::add(hwnd, window::WM_APP_TRAYICON, hicon, tooltip)?;
    Ok(())
}
```

- [ ] **Step 6: Verify it compiles**

Run: `cargo build`
Expected: clean build. Likely errors:
- Unused import of `IconSet` if you forgot to update line 1. Fix by changing it to `IconRenderer`.
- "`icons` field not found on TrayState" — find any remaining reference, replace.

- [ ] **Step 7: Verify tests still pass**

Run: `cargo test --lib`
Expected: all tests pass.

- [ ] **Step 8: Smoke test — run the tray app**

Run: `cargo run`
Expected:
- A tray icon appears (you may need to click the `^` overflow arrow on Win11 — see [`reference_win11_tray_overflow.md`](~/.claude/projects/C--Users-borgi-Documents-claude-usage-tray/memory/reference_win11_tray_overflow.md)).
- Initial icon is solid gray (the `?` glyph isn't drawn yet — that's Task 7).
- After the first successful poll, the icon switches to a solid color from the anchored gradient (green-ish if util is low, yellow/orange/red if higher).
- Hover for tooltip — same Stage 3 format.
- Right-click → Quit — icon disappears, process exits.

- [ ] **Step 9: Commit**

```
git add src/tray/icon.rs src/tray/window.rs src/tray/mod.rs
git commit -m "feat(stage-4): wire IconRenderer into TrayState with HICON lifecycle"
```

---

## Task 7: Add glyph rendering inside `IconRenderer::render`

Now the tray app runs with flat-colored icons. Add the text rendering on top: white digits / `!` / `?` with a 1-pixel black outline, centered in the 16×16 bitmap.

**Files:**
- Modify: `src/tray/icon.rs` — extend `IconRenderer::render` with glyph drawing

- [ ] **Step 1: Add the additional imports**

Append to the GDI+ import block in `src/tray/icon.rs`:

```rust
use windows::Win32::Graphics::GdiPlus::{
    // existing items kept
    FillModeAlternate, FontStyleBold, GdipAddPathString, GdipCreateFontFamilyFromName,
    GdipCreatePath, GdipCreatePen1, GdipCreateSolidFill, GdipCreateStringFormat,
    GdipDeleteBrush, GdipDeleteFontFamily, GdipDeletePath, GdipDeletePen,
    GdipDeleteStringFormat, GdipDrawPath, GdipFillPath, GdipSetStringFormatAlign,
    GdipSetStringFormatLineAlign, GdipSetTextRenderingHint, GpBrush, GpFontFamily, GpPath,
    GpPen, GpSolidFill, GpStringFormat, RectF, StringAlignmentCenter, TextRenderingHintAntiAliasGridFit,
    UnitPixel,
};
```

(Some of these may be in slightly different submodules — `cargo check` will tell you. The shape is `windows::Win32::Graphics::GdiPlus::<NAME>`.)

- [ ] **Step 2: Add a `glyph_to_string` helper**

Above `impl IconRenderer`, add:

```rust
/// UTF-16 null-terminated text for a Glyph. Returns the (string, length-in-chars) pair.
fn glyph_to_text(glyph: Glyph) -> (Vec<u16>, i32) {
    let s = match glyph {
        Glyph::Digits(n) => format!("{n}"),
        Glyph::Bang => "!".to_string(),
        Glyph::Question => "?".to_string(),
    };
    let len = s.chars().count() as i32;
    let utf16: Vec<u16> = s.encode_utf16().chain(std::iter::once(0)).collect();
    (utf16, len)
}
```

- [ ] **Step 3: Replace the `// TODO Task 7` block with the glyph-drawing code**

In `IconRenderer::render`, replace the `// TODO Task 7: draw the glyph here.` comment with:

```rust
        // Glyph drawing
        let (text, text_len) = glyph_to_text(_glyph);

        // Enable AA text rendering for the path.
        // SAFETY: graphics is valid; hint is a documented enum value.
        unsafe { GdipSetTextRenderingHint(graphics, TextRenderingHintAntiAliasGridFit) };

        // Create FontFamily ("Segoe UI" — Black weight requested via FontStyleBold below).
        let font_name: Vec<u16> = "Segoe UI"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let mut family: *mut GpFontFamily = std::ptr::null_mut();
        // SAFETY: name buffer is null-terminated; system font collection (null) is the default.
        let s = unsafe {
            GdipCreateFontFamilyFromName(font_name.as_ptr(), std::ptr::null_mut(), &mut family)
        };
        if s != Status(0) {
            unsafe {
                windows::Win32::Graphics::GdiPlus::GdipDeleteGraphics(graphics);
                GdipDisposeImage(bitmap as *mut GpImage);
            }
            anyhow::bail!("GdipCreateFontFamilyFromName failed: {s:?}");
        }

        // Create a centered StringFormat.
        let mut fmt: *mut GpStringFormat = std::ptr::null_mut();
        // SAFETY: out-pointer valid.
        unsafe { GdipCreateStringFormat(0, 0, &mut fmt) };
        unsafe { GdipSetStringFormatAlign(fmt, StringAlignmentCenter) };
        unsafe { GdipSetStringFormatLineAlign(fmt, StringAlignmentCenter) };

        // Layout rect = full 16x16 bitmap.
        let layout = RectF { X: 0.0, Y: 0.0, Width: 16.0, Height: 16.0 };

        // Build a GraphicsPath for the glyph text.
        let mut path: *mut GpPath = std::ptr::null_mut();
        unsafe { GdipCreatePath(FillModeAlternate, &mut path) };

        // Em-size 12 is a starting point. Tune in implementation if too big/small.
        // SAFETY: all pointers valid; text length is correct.
        unsafe {
            GdipAddPathString(
                path,
                text.as_ptr(),
                text_len,
                family,
                FontStyleBold as i32,
                12.0,
                &layout,
                fmt,
            );
        }

        // Draw outline: 1px black Pen along the path.
        let mut pen: *mut GpPen = std::ptr::null_mut();
        // ARGB: 0xFF000000 = opaque black. Width 1.0 pixel.
        unsafe { GdipCreatePen1(0xFF00_0000u32 as i32, 1.0, UnitPixel, &mut pen) };
        unsafe { GdipDrawPath(graphics, pen, path) };

        // Fill interior: white SolidBrush.
        let mut brush: *mut GpSolidFill = std::ptr::null_mut();
        unsafe { GdipCreateSolidFill(0xFFFF_FFFFu32 as i32, &mut brush) };
        unsafe { GdipFillPath(graphics, brush as *mut GpBrush, path) };

        // Clean up glyph-specific GDI+ objects.
        unsafe {
            GdipDeleteBrush(brush as *mut GpBrush);
            GdipDeletePen(pen);
            GdipDeletePath(path);
            GdipDeleteStringFormat(fmt);
            GdipDeleteFontFamily(family);
        }
```

Also remove the leading `_` from `_glyph` (rename to `glyph`) since we're using it now:

```rust
let ((r, g, b), glyph) = compute_visual(status, sample);
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo build`
Expected: clean. Most likely failure mode: a `Gdip*` function name or argument type doesn't match the `windows` crate's binding. For each error, look up the actual signature via rust-analyzer / `cargo doc` and adjust.

- [ ] **Step 5: Verify tests still pass**

Run: `cargo test --lib`
Expected: all tests pass.

- [ ] **Step 6: Smoke test — visual check**

Run: `cargo run`
Expected:
- Initial tray icon: solid gray with white "?".
- After first successful poll: gradient-colored bg with white 1- or 2-digit number, black outline.
- If your util crosses a threshold mid-session (you'll need to manually generate load), the color transitions smoothly.
- Tooltip still shows both 5h and 7d.

If the text looks muddy/illegible:
- Try a different `em_size` (10.0, 11.0, 14.0) on the `GdipAddPathString` call.
- Try `TextRenderingHintSingleBitPerPixel` instead of `AntiAliasGridFit` for crisper pixels (no AA).
- Try `FontStyleRegular` instead of `FontStyleBold` if the bold is too heavy at 12em on 16×16.

Iterate until it looks right. Commit each tweak.

- [ ] **Step 7: Commit**

```
git add src/tray/icon.rs
git commit -m "feat(stage-4): render glyph (digits / ! / ?) on the icon background"
```

---

## Task 8: Remove the dead `IconSet` code

`IconRenderer` is fully wired up. The old `IconSet` struct and its `solid_icon` helper are unreachable. Delete them so future readers don't get confused.

**Files:**
- Modify: `src/tray/icon.rs` — delete `IconSet` struct, `impl IconSet`, `impl Drop for IconSet`, `solid_icon` function

- [ ] **Step 1: Delete the `IconSet` code block**

In `src/tray/icon.rs`, delete these items in their entirety:

- `pub struct IconSet { ... }` and its doc comment.
- `impl IconSet { ... }` (the whole block with `new` and `for_state`).
- `impl Drop for IconSet { ... }`.
- `fn solid_icon(...)` (the private helper).

Keep:
- `pub fn add`, `pub fn modify`, `pub fn delete`, `fn base_notify_data` (these wrap Shell_NotifyIconW and don't depend on IconSet).
- `IconRenderer` + helpers (Task 5–7).
- Pure-function helpers (`anchored_gradient`, `compute_visual`, `util_max`, `percent_int`, `glyph_to_text`, `Glyph`).
- The tests block.

- [ ] **Step 2: Check for orphan imports**

If `CreateIcon` is now imported but unused, remove it from the `use` line at the top of `src/tray/icon.rs`. `cargo build` will warn about unused imports.

- [ ] **Step 3: Verify it compiles**

Run: `cargo build`
Expected: clean. No warnings.

- [ ] **Step 4: Run clippy**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: clean (no warnings — the project conventions require this before tagging).

- [ ] **Step 5: Verify tests still pass**

Run: `cargo test --lib`
Expected: all tests pass.

- [ ] **Step 6: Commit**

```
git add src/tray/icon.rs
git commit -m "refactor(stage-4): remove dead IconSet code (superseded by IconRenderer)"
```

---

## Task 9: End-to-end smoke test, formatting, and v0.4.0 release

The implementation is complete. This task runs the full pre-release checklist from the design spec.

**Files:**
- Modify: `Cargo.toml` (version bump)
- Modify: `CLAUDE.md` (mark Stage 4 as shipped)

- [ ] **Step 1: `cargo fmt`**

Run: `cargo fmt --all`
Then: `cargo fmt --all -- --check`
Expected: clean exit (no diff after the format pass).

- [ ] **Step 2: `cargo clippy`**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: 0 warnings, 0 errors.

- [ ] **Step 3: Full test run**

Run: `cargo test`
Expected: all tests pass (existing 13 from Stages 1–3 + ~8 new from `compute_visual` + ~5 from `anchored_gradient` ≈ 26 total).

- [ ] **Step 4: Release build**

Run: `cargo build --release`
Expected: clean. Binary at `target\release\claude-usage-tray.exe`, ~4.5 MB.

- [ ] **Step 5: Long-running smoke test (handle leak check)**

Launch the release build:
```
.\target\release\claude-usage-tray.exe
```

Open Process Explorer (or `Get-Process claude-usage-tray | Select-Object Handles, NPM, PM, WS`), note the handle count. Let the app run for ≥10 polling intervals (~20 minutes at default --interval 120). Re-check the handle count.

Expected: handle count is stable (variance ±5 is normal Windows noise). If it grows linearly with poll count, the HICON destruction path is broken — debug before tagging.

- [ ] **Step 6: Force-error smoke tests**

While the tray is running:

1. Trigger a rate limit: open a terminal and run `.\target\release\claude-usage-tray.exe --once` twice within 60s. The second one should fail with HTTP 429. The tray icon should go gray + "?" on its next poll tick.
2. Verify return to Ok: wait one interval. The tray should poll successfully again and return to the colored gradient.
3. Verify shutdown: right-click the tray icon, click Quit. The icon disappears immediately. The process exits within ~1s.

- [ ] **Step 7: Verify --once and --watch still work**

Run: `.\target\release\claude-usage-tray.exe --once`
Expected: prints `5h: NN%` / `7d: NN%` lines (or `(no data)` if not authenticated). Same Stage 2 behavior.

Run: `.\target\release\claude-usage-tray.exe --watch` for 30s, then Ctrl+C.
Expected: live updates every interval, clean Ctrl+C exit.

- [ ] **Step 8: Bump version in Cargo.toml**

Find:
```toml
version = "0.3.0"
```

Replace with:
```toml
version = "0.4.0"
```

- [ ] **Step 9: Update CLAUDE.md**

In `CLAUDE.md`:

In the "Stage roadmap (summary — see spec for details)" table, change:
```
| 4 | GDI-rendered percentage icon | Pending |
```
to:
```
| 4 | GDI-rendered percentage icon | ✅ Shipped — tag `v0.4.0`, pushed to GitHub |
```

In the "Active design + plans" list, change:
```
- **Stage 4 spec:** `docs/superpowers/specs/2026-05-23-stage-4-gdi-icon-design.md` — GDI+ rendered percentage icon design details.
```
to:
```
- **Stage 4 spec:** `docs/superpowers/specs/2026-05-23-stage-4-gdi-icon-design.md` — GDI+ rendered percentage icon design details.
- **Stage 4 plan:** `docs/superpowers/plans/2026-05-23-stage-4-gdi-icon.md` — task plan. **Shipped 2026-05-23 (tag `v0.4.0`).**
```

- [ ] **Step 10: Final format / clippy / test cycle**

Run, in this order:
```
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
```
Expected: all clean.

- [ ] **Step 11: Commit the release bump**

```
git add Cargo.toml Cargo.lock CLAUDE.md
git commit -m "release: bump version to 0.4.0"
```

- [ ] **Step 12: Tag and push**

```
git tag -a v0.4.0 -m "Stage 4: GDI+ rendered percentage icon"
git push origin main
git push origin v0.4.0
```

- [ ] **Step 13: Verify on GitHub**

Open https://github.com/borgi-s/claude-usage-tray/tags. The `v0.4.0` tag should appear with its message.

Open https://github.com/borgi-s/claude-usage-tray. The README should still render. The Tags tab shows v0.1.0, v0.2.0, v0.3.0, v0.4.0.

- [ ] **Step 14: Prune the brainstorm branch (optional)**

If you're satisfied with the spec captures, the `design/stage-4-mockups` branch can be deleted:

```
git push origin --delete design/stage-4-mockups
git branch -D design/stage-4-mockups
```

This removes the SVG mockup commits from the remote. They served their purpose during brainstorming; the design spec on `main` is the canonical record.

(Or keep the branch indefinitely as design rationale — your call. The mockups don't affect anything on `main`.)
