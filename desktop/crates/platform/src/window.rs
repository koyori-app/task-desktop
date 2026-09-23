#[cfg(windows)]
use crate::error::Error;
use crate::error::Result;

/// GPUI's Windows app.hide() is currently a no-op. Use only the native handle
/// supplied by the caller's live window; never infer a foreground/global HWND.
/// Returns false when the caller must use GPUI's platform fallback: hide the
/// application on macOS, minimize the window on Linux, and activate to restore.
/// Linux's application-wide hide() is currently a no-op in GPUI.
pub fn set_window_visible(
    window: &impl raw_window_handle::HasWindowHandle,
    visible: bool,
) -> Result<bool> {
    #[cfg(windows)]
    {
        use raw_window_handle::RawWindowHandle;
        use windows::Win32::{
            Foundation::HWND,
            UI::WindowsAndMessaging::{SW_HIDE, SW_RESTORE, SetForegroundWindow, ShowWindow},
        };
        let handle = window
            .window_handle()
            .map_err(|e| Error::Other(e.to_string()))?;
        let RawWindowHandle::Win32(handle) = handle.as_raw() else {
            return Ok(false);
        };
        let hwnd = HWND(handle.hwnd.get() as *mut std::ffi::c_void);
        // The borrowed window handle guarantees this HWND remains live during
        // these UI-thread calls. ShowWindow's return value is previous visibility.
        unsafe {
            let _ = ShowWindow(hwnd, if visible { SW_RESTORE } else { SW_HIDE });
            if visible {
                let _ = SetForegroundWindow(hwnd);
            }
        }
        Ok(true)
    }
    #[cfg(not(windows))]
    {
        let _ = (window, visible);
        Ok(false)
    }
}
