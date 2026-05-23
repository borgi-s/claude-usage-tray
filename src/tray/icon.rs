use crate::api::usage::UsageSnapshot;
use crate::render::LastStatus;
use anyhow::{anyhow, Result};
use windows::Win32::Foundation::{HMODULE, HWND};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{CreateIcon, DestroyIcon, HICON};

/// Four pre-rendered solid-color tray icons, allocated once at startup and
/// reused across renders. Released via `Drop` (calls `DestroyIcon` on each).
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
#[allow(dead_code)]
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
