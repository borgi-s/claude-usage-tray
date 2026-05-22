//! Win32 tray icon mode. Entry point: [`run`].
//!
//! Threading: caller (main thread) runs the Win32 message loop. A polling
//! thread spawned by `run` sends `PollEvent`s via mpsc and wakes the UI thread
//! with `PostMessageW(hwnd, WM_APP+1, ...)`.

pub mod icon;
pub mod poller;
pub mod window;

use anyhow::Result;

/// Run the tray app. Blocks until the user clicks Quit or the process is
/// otherwise terminated. Returns `Ok(())` on clean shutdown.
pub fn run(_interval_secs: u64) -> Result<()> {
    anyhow::bail!("tray::run not yet implemented (Task 8 wires this up)")
}
