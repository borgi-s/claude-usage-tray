use crate::api::usage::UsageSnapshot;
use crate::render::LastStatus;
use anyhow::{anyhow, Result};
use windows::Win32::Foundation::{HMODULE, HWND};
use windows::Win32::Graphics::GdiPlus::{
    FillModeAlternate, FontStyleBold, GdipAddPathString, GdipCreateFontFamilyFromName,
    GdipCreatePath, GdipCreatePen1, GdipCreateSolidFill, GdipCreateStringFormat,
    GdipDeleteBrush, GdipDeleteFontFamily, GdipDeletePath, GdipDeletePen,
    GdipDeleteStringFormat, GdipDrawPath, GdipFillPath, GdipSetStringFormatAlign,
    GdipSetStringFormatLineAlign, GdipSetTextRenderingHint,
    GdipCreateBitmapFromScan0, GdipCreateHICONFromBitmap, GdipDeleteGraphics, GdipDisposeImage,
    GdipGetImageGraphicsContext, GdipGraphicsClear,
    GpBitmap, GpBrush, GpFontFamily, GpGraphics, GpImage, GpPath, GpPen, GpSolidFill,
    GpStringFormat, RectF, Status,
    StringAlignmentCenter, TextRenderingHintAntiAliasGridFit, UnitPixel,
};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{CreateIcon, DestroyIcon, HICON};

// PixelFormat32bppARGB is not exported as a named constant in windows-0.58.
// Value: (10 | (32 << 8) | PixelFormatAlpha | PixelFormatGDI | PixelFormatCanonical)
//       = 10 | 8192 | 262144 | 131072 | 2097152 = 0x0026200A
const PIXEL_FORMAT_32BPP_ARGB: i32 = 0x0026_200A;

/// Four pre-rendered solid-color tray icons, allocated once at startup and
/// reused across renders. Released via `Drop` (calls `DestroyIcon` on each).
/// NOTE: unused since Task 6 wired in IconRenderer; will be removed in Task 8.
#[allow(dead_code)]
pub struct IconSet {
    pub gray: HICON,
    pub green: HICON,
    pub yellow: HICON,
    pub red: HICON,
}

impl IconSet {
    /// Build all four icons. Fails if any `CreateIcon` call returns null.
    pub fn new(hinst: HMODULE) -> Result<Self> {
        Ok(Self {
            gray: solid_icon(hinst, 0x80, 0x80, 0x80)?,
            green: solid_icon(hinst, 0x2E, 0xB8, 0x2E)?,
            yellow: solid_icon(hinst, 0xE6, 0xB8, 0x00)?, // RGB; build_buffer reorders to BGRA
            red: solid_icon(hinst, 0xCC, 0x29, 0x29)?,
        })
    }

    /// Pick the icon that matches the current state and most-recent sample.
    /// `sample` is the most recent SUCCESSFUL snapshot; status is the most
    /// recent poll outcome (which may be RateLimited/Error even when we have
    /// a cached `sample`).
    pub fn for_state(&self, status: &LastStatus, sample: Option<&UsageSnapshot>) -> HICON {
        match status {
            LastStatus::Initial => self.gray,
            LastStatus::RateLimited | LastStatus::Error(_) => self.gray,
            LastStatus::Ok => {
                let util = sample
                    .map(|s| {
                        let h5 = s.five_hour.as_ref().map(|b| b.utilization).unwrap_or(0.0);
                        let d7 = s.seven_day.as_ref().map(|b| b.utilization).unwrap_or(0.0);
                        h5.max(d7)
                    })
                    .unwrap_or(0.0);
                if util < 0.60 {
                    self.green
                } else if util < 0.85 {
                    self.yellow
                } else {
                    self.red
                }
            }
        }
    }
}

impl Drop for IconSet {
    fn drop(&mut self) {
        // SAFETY: each HICON was created by CreateIcon and we own them.
        unsafe {
            let _ = DestroyIcon(self.gray);
            let _ = DestroyIcon(self.green);
            let _ = DestroyIcon(self.yellow);
            let _ = DestroyIcon(self.red);
        }
    }
}

/// Build a 16x16 solid-color HICON. RGB inputs are reordered to the BGRA byte
/// order that CreateIcon expects for 32bpp bitmaps.
/// NOTE: unused since Task 6; will be removed in Task 8.
#[allow(dead_code)]
fn solid_icon(hinst: HMODULE, r: u8, g: u8, b: u8) -> Result<HICON> {
    let mut color = [0u8; 16 * 16 * 4];
    let mut i = 0;
    while i < color.len() {
        color[i] = b;
        color[i + 1] = g;
        color[i + 2] = r;
        color[i + 3] = 0xFF;
        i += 4;
    }
    // Mask: 32 bytes = 16x16 bits, all zero means "use the color buffer's pixels".
    let mask = [0u8; 32];

    // SAFETY: buffers outlive the call; sizes match arg values.
    // In windows-0.58+, CreateIcon returns Result<HICON> rather than a raw HICON.
    let hicon = unsafe { CreateIcon(hinst, 16, 16, 1, 32, mask.as_ptr(), color.as_ptr()) }
        .map_err(|e| anyhow!("CreateIcon failed: {}", e))?;

    Ok(hicon)
}

/// Add the tray icon to the shell on startup.
pub fn add(hwnd: HWND, callback_msg: u32, icon: HICON, tooltip: &[u16]) -> Result<()> {
    let data = base_notify_data(hwnd, callback_msg, icon, tooltip);
    // SAFETY: data is on the stack and lives for the duration of the call.
    let ok = unsafe { Shell_NotifyIconW(NIM_ADD, &data) };
    if ok.as_bool() {
        Ok(())
    } else {
        Err(anyhow!("Shell_NotifyIcon NIM_ADD failed"))
    }
}

/// Update the tray icon's icon and tooltip after each poll.
pub fn modify(hwnd: HWND, callback_msg: u32, icon: HICON, tooltip: &[u16]) {
    let data = base_notify_data(hwnd, callback_msg, icon, tooltip);
    // SAFETY: data is on the stack and lives for the duration of the call.
    let ok = unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) };
    if !ok.as_bool() {
        tracing::warn!("Shell_NotifyIcon NIM_MODIFY failed (Explorer restarted?)");
    }
}

/// Remove the tray icon on shutdown.
pub fn delete(hwnd: HWND) {
    let data = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        ..Default::default()
    };
    // SAFETY: data is on the stack and lives for the duration of the call.
    unsafe {
        let _ = Shell_NotifyIconW(NIM_DELETE, &data);
    }
}

fn base_notify_data(
    hwnd: HWND,
    callback_msg: u32,
    icon: HICON,
    tooltip: &[u16],
) -> NOTIFYICONDATAW {
    let mut data = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        uFlags: NIF_ICON | NIF_TIP | NIF_MESSAGE,
        uCallbackMessage: callback_msg,
        hIcon: icon,
        ..Default::default()
    };
    // Copy tooltip UTF-16 into szTip (128-element wchar buffer). Truncate to fit.
    let n = tooltip.len().min(data.szTip.len() - 1);
    data.szTip[..n].copy_from_slice(&tooltip[..n]);
    // Ensure null-terminator.
    data.szTip[n] = 0;
    data
}

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

/// UTF-16 null-terminated text for a Glyph. Returns the (string, length-in-chars) pair.
/// The `length` is the count of meaningful UTF-16 code units (excluding the null terminator),
/// which is what GdipAddPathString expects for its `length` parameter.
fn glyph_to_text(glyph: Glyph) -> (Vec<u16>, i32) {
    let s = match glyph {
        Glyph::Digits(n) => format!("{n}"),
        Glyph::Bang => "!".to_string(),
        Glyph::Question => "?".to_string(),
    };
    // GdipAddPathString uses the `length` to know how many UTF-16 code units to render.
    // ASCII digits, '!' and '?' are all in BMP so each char = one code unit.
    let len = s.encode_utf16().count() as i32;
    // Append null terminator for the PCWSTR buffer (required by GDI+).
    let utf16: Vec<u16> = s.encode_utf16().chain(std::iter::once(0)).collect();
    (utf16, len)
}

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
        let ((r, g, b), glyph) = compute_visual(status, sample);

        // 1) Create a 16x16 ARGB bitmap.
        // SAFETY: out-pointer is valid; stride 0 means "let GDI+ pick"; scan0 None means
        // "allocate a fresh buffer." PIXEL_FORMAT_32BPP_ARGB requests premultiplied ARGB.
        let mut bitmap: *mut GpBitmap = std::ptr::null_mut();
        let s = unsafe {
            GdipCreateBitmapFromScan0(16, 16, 0, PIXEL_FORMAT_32BPP_ARGB, None, &mut bitmap)
        };
        if s != Status(0) {
            anyhow::bail!("GdipCreateBitmapFromScan0 failed: {s:?}");
        }

        // 2) Get a Graphics for that bitmap.
        // SAFETY: bitmap was just created successfully; cast to GpImage is the GDI+ idiom.
        let mut graphics: *mut GpGraphics = std::ptr::null_mut();
        let s = unsafe {
            GdipGetImageGraphicsContext(bitmap as *mut GpImage, &mut graphics)
        };
        if s != Status(0) {
            // SAFETY: bitmap is valid; we own it.
            unsafe { GdipDisposeImage(bitmap as *mut GpImage); }
            anyhow::bail!("GdipGetImageGraphicsContext failed: {s:?}");
        }

        // 3) Fill background. GDI+ Argb format is 0xAARRGGBB.
        let argb = 0xFF00_0000u32 | (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b);
        // SAFETY: graphics is valid; argb is u32 (GdipGraphicsClear takes u32).
        let s = unsafe { GdipGraphicsClear(graphics, argb) };
        if s != Status(0) {
            // SAFETY: both handles valid.
            unsafe {
                GdipDeleteGraphics(graphics);
                GdipDisposeImage(bitmap as *mut GpImage);
            }
            anyhow::bail!("GdipGraphicsClear failed: {s:?}");
        }

        // 4a) Glyph drawing: white digits / ! / ? with 1-pixel black outline.
        let (text, text_len) = glyph_to_text(glyph);

        // Enable anti-aliased text rendering for the GraphicsPath.
        // SAFETY: graphics is valid; hint is a documented enum value.
        unsafe { GdipSetTextRenderingHint(graphics, TextRenderingHintAntiAliasGridFit) };

        // Create FontFamily for "Segoe UI". Null font collection = system default.
        let font_name: Vec<u16> = "Segoe UI"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let mut family: *mut GpFontFamily = std::ptr::null_mut();
        // SAFETY: font_name is null-terminated; null collection uses the system font collection.
        // GdipCreateFontFamilyFromName accepts P0: Param<PCWSTR>; PCWSTR wraps a *const u16.
        let s = unsafe {
            GdipCreateFontFamilyFromName(
                windows::core::PCWSTR(font_name.as_ptr()),
                std::ptr::null_mut(),
                &mut family,
            )
        };
        if s != Status(0) {
            unsafe {
                GdipDeleteGraphics(graphics);
                GdipDisposeImage(bitmap as *mut GpImage);
            }
            anyhow::bail!("GdipCreateFontFamilyFromName failed: {s:?}");
        }

        // Create a centered StringFormat (layout: centered horizontally and vertically).
        let mut fmt: *mut GpStringFormat = std::ptr::null_mut();
        // SAFETY: out-pointer is valid. Language 0 = neutral locale.
        unsafe { GdipCreateStringFormat(0, 0u16, &mut fmt) };
        unsafe { GdipSetStringFormatAlign(fmt, StringAlignmentCenter) };
        unsafe { GdipSetStringFormatLineAlign(fmt, StringAlignmentCenter) };

        // Layout rect = full 16×16 bitmap.
        let layout = RectF { X: 0.0, Y: 0.0, Width: 16.0, Height: 16.0 };

        // Build a GraphicsPath that traces the glyph outline.
        let mut path: *mut GpPath = std::ptr::null_mut();
        // SAFETY: out-pointer valid; FillModeAlternate is the standard winding rule.
        unsafe { GdipCreatePath(FillModeAlternate, &mut path) };

        // Add the glyph text to the path. Em-size 11 works well for 1-2 chars at 16×16.
        // FontStyleBold.0 unwraps the FontStyle newtype to i32 for the style parameter.
        // SAFETY: text is null-terminated PCWSTR; text_len is correct code-unit count.
        unsafe {
            GdipAddPathString(
                path,
                windows::core::PCWSTR(text.as_ptr()),
                text_len,
                family,
                FontStyleBold.0,
                11.0f32,
                &layout,
                fmt,
            )
        };

        // Draw 1-pixel black outline along the path.
        let mut pen: *mut GpPen = std::ptr::null_mut();
        // ARGB 0xFF000000 = opaque black. UnitPixel means width is in screen pixels.
        // SAFETY: out-pointer valid; color and unit are valid enum values.
        unsafe { GdipCreatePen1(0xFF00_0000u32, 1.0f32, UnitPixel, &mut pen) };
        // SAFETY: graphics, pen, and path are all valid.
        unsafe { GdipDrawPath(graphics, pen, path) };

        // Fill glyph interior with white.
        let mut brush: *mut GpSolidFill = std::ptr::null_mut();
        // ARGB 0xFFFFFFFF = opaque white.
        // SAFETY: out-pointer valid; color is a valid ARGB value.
        unsafe { GdipCreateSolidFill(0xFFFF_FFFFu32, &mut brush) };
        // SAFETY: graphics, brush (cast to GpBrush), and path are valid.
        unsafe { GdipFillPath(graphics, brush as *mut GpBrush, path) };

        // Clean up glyph-specific GDI+ objects (LIFO order; error paths may leak these
        // on early-exit before this point — accepted simplification for Stage 4 MVP).
        // SAFETY: all handles were successfully created above.
        unsafe {
            GdipDeleteBrush(brush as *mut GpBrush);
            GdipDeletePen(pen);
            GdipDeletePath(path);
            GdipDeleteStringFormat(fmt);
            GdipDeleteFontFamily(family);
        }

        // 4) Convert to HICON. The HICON owns its pixel data independently
        //    from the bitmap, so we can dispose the bitmap right after.
        let mut hicon = HICON::default();
        // SAFETY: bitmap valid; hicon out-pointer valid.
        let s = unsafe { GdipCreateHICONFromBitmap(bitmap, &mut hicon) };

        // 5) Clean up GDI+ objects regardless of conversion outcome.
        // SAFETY: both handles valid.
        unsafe {
            GdipDeleteGraphics(graphics);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::usage::UsageBucket;

    fn snap_with(five: Option<f64>, seven: Option<f64>) -> UsageSnapshot {
        UsageSnapshot {
            five_hour: five.map(|u| UsageBucket { utilization: u, resets_at: None }),
            seven_day: seven.map(|u| UsageBucket { utilization: u, resets_at: None }),
        }
    }

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
}
